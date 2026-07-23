//! System-audio capture on Windows via WASAPI.
//!
//! Primary path is **process loopback** (`ActivateAudioInterfaceAsync` with
//! `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK`, exclude-self): it taps every other
//! process's render stream upstream of the endpoint mixer, so it keeps recording
//! even when the speaker is muted and never captures our own sounds. If process
//! loopback can't activate (older Windows / error) we fall back to classic
//! endpoint loopback on the default render device.
//!
//! This module only *captures*: it timestamps each buffer and sends an
//! [`AudioPacket`] onto the shared queue. Decoding to f32 is unavoidable format
//! normalization; everything else (mixing, VAD, storage) is the pipeline's job.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, IUnknown, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eRender, ERole, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::audio::{AudioPacket, AudioSource};

const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const VT_BLOB: u16 = 65;
/// Format we request from process loopback (the virtual device has no mix format).
const CAPTURE_RATE: u32 = 48_000;

/// Which kind of WASAPI stream a capture thread should open.
#[derive(Clone, Copy)]
enum StreamKind {
    /// System output via loopback (labeled "Remote").
    System,
    /// Local microphone via a WASAPI capture endpoint (labeled "You").
    Microphone,
}

/// Start system-audio (loopback) capture on a dedicated thread, sending packets
/// to `tx`. Blocks until the stream is running. Returns the thread handle plus
/// `endpoint_fallback = true` when it had to use classic endpoint loopback (which,
/// unlike process loopback, is bound to one device and goes silent when the
/// endpoint is muted) — so the caller can watch for mute on that path.
pub fn start(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<(JoinHandle<()>, bool)> {
    start_with(tx, stop, StreamKind::System, "meetapp-system-audio")
        .map(|(join, info)| (join, info.endpoint_fallback))
}

/// Whether the default render endpoint (speakers/headphones) is muted right now.
/// `None` if it can't be determined. This only matters on the endpoint-loopback
/// fallback: process loopback taps *upstream* of the endpoint mixer, so it keeps
/// capturing regardless of mute. Read-only — it never changes the mute state.
///
/// Queries the **communications** render device first, then the **console**
/// default — matching the endpoint-loopback fallback's device preference, so the
/// mute we report is the mute of the endpoint we're actually capturing rather than
/// a different default device.
pub fn default_render_muted() -> Option<bool> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eCommunications)
            .or_else(|_| enumerator.GetDefaultAudioEndpoint(eRender, eConsole))
            .ok()?;
        let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        vol.GetMute().ok().map(|b| b.as_bool())
    }
}

/// Whether the default **capture** endpoint (the microphone) is muted right now.
/// `None` if it can't be determined. Mirrors [`default_render_muted`] for the input
/// side: it reports the OS/hardware mic mute — the Windows Sound-settings input
/// mute, a headset's mute switch, or a keyboard mic-mute key, all of which toggle
/// this endpoint mute. Used to stop recording the microphone while the user has it
/// muted (system audio keeps recording). We query the **communications** capture
/// device first — the one the mic capture opens (see [`setup_capture_endpoint`]) —
/// then the console default, so the mute we read is the mute of the device we're
/// actually recording. Read-only — it never changes the mute state.
///
/// Note: this detects an OS/device-level mute only. Muting yourself *inside* a
/// meeting app (the Teams/Zoom mute button) does not mute the Windows endpoint and
/// is not exposed by any public API, so it cannot be detected here.
pub fn default_capture_muted() -> Option<bool> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eCommunications)
            .or_else(|_| enumerator.GetDefaultAudioEndpoint(eCapture, eConsole))
            .ok()?;
        let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        vol.GetMute().ok().map(|b| b.as_bool())
    }
}

/// Start microphone capture through a raw WASAPI capture endpoint. This is the
/// fallback the mic uses when cpal can't open the default input device. Returns
/// the capture thread plus `exclusive = true` when it had to open the device in
/// exclusive mode (so the UI can warn about the meeting-app conflict that implies).
pub fn start_microphone(
    tx: Sender<AudioPacket>,
    stop: Arc<AtomicBool>,
) -> Option<(JoinHandle<()>, bool)> {
    start_with(tx, stop, StreamKind::Microphone, "meetapp-wasapi-mic")
        .map(|(join, info)| (join, info.exclusive))
}

fn start_with(
    tx: Sender<AudioPacket>,
    stop: Arc<AtomicBool>,
    kind: StreamKind,
    thread_name: &str,
) -> Option<(JoinHandle<()>, CaptureInfo)> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<CaptureInfo, String>>();
    let join = std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            if let Err(e) = run(&stop, &tx, &ready_tx, kind) {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        })
        .ok()?;

    // Setup can now include format negotiation + a short delivery check, so allow
    // a little longer than a plain Initialize would need.
    match ready_rx.recv_timeout(Duration::from_secs(12)) {
        Ok(Ok(info)) => Some((join, info)),
        Ok(Err(e)) => {
            tracing::warn!("WASAPI capture unavailable ({thread_name}): {e}");
            let _ = join.join();
            None
        }
        Err(_) => {
            tracing::warn!("WASAPI capture setup timed out ({thread_name})");
            None
        }
    }
}

fn run(
    stop: &AtomicBool,
    tx: &Sender<AudioPacket>,
    ready: &mpsc::Sender<Result<CaptureInfo, String>>,
    kind: StreamKind,
) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // Supervised capture loop. A recording outlives any single audio device:
        // when the user plugs in / switches the output (or the mic) mid-meeting,
        // WASAPI invalidates the stream we opened against the old endpoint. Rather
        // than die silently (the old bug — capture would spin on a dead endpoint and
        // record nothing for the rest of the session), we tear down and re-open,
        // re-querying the *current* default device so capture follows the switch.
        //
        // Process loopback (the preferred system path) is device-agnostic and
        // normally never invalidates, so this loop is a no-op for it; it exists to
        // make the endpoint-loopback fallback and the mic robust to device changes.
        let mut announced = false;
        while !stop.load(Ordering::Relaxed) {
            let stream = match open_stream(kind) {
                Ok(s) => s,
                Err(e) => {
                    // The very first open is the caller's success/fail signal (it may
                    // fall back to cpal). A later open failing means a device is still
                    // settling after a switch — back off and keep trying until stop.
                    if !announced {
                        return Err(e);
                    }
                    tracing::warn!("system audio: re-open failed ({e}); retrying");
                    if sleep_unless_stopped(stop, Duration::from_millis(300)) {
                        break;
                    }
                    continue;
                }
            };

            if let Err(e) = stream.client.Start() {
                let _ = stream.client.Stop();
                if !announced {
                    return Err(e.into());
                }
                tracing::warn!("system audio: Start failed on re-open ({e}); retrying");
                if sleep_unless_stopped(stop, Duration::from_millis(300)) {
                    break;
                }
                continue;
            }

            // For the microphone, confirm the callback actually delivers PCM before we
            // report success. A stream can initialize yet never fire — that used to
            // look like a working mic and record silence; instead we now fail so the
            // caller falls through to the next path (e.g. cpal). Loopback is exempt: it
            // legitimately delivers nothing while the speakers are idle. Only gate the
            // *first* open on this — a re-open after a device change is best-effort.
            if matches!(kind, StreamKind::Microphone)
                && !announced
                && !validate_capture(&stream, tx, Duration::from_millis(1500))
            {
                let _ = stream.client.Stop();
                return Err("capture endpoint opened but delivered no PCM".into());
            }

            if !announced {
                announced = true;
                let _ = ready.send(Ok(CaptureInfo {
                    exclusive: stream.exclusive,
                    endpoint_fallback: stream.endpoint_fallback,
                }));
            }

            let outcome = pump(&stream, stop, tx);
            let _ = stream.client.Stop();
            // `stream` drops here → its Drop closes the event handle.

            match outcome {
                PumpOutcome::Stopped => break,
                PumpOutcome::DeviceChanged => {
                    tracing::info!(
                        "system audio: audio device changed — re-opening capture on the new default device"
                    );
                    // Give the new default endpoint a moment to become ready.
                    if sleep_unless_stopped(stop, Duration::from_millis(150)) {
                        break;
                    }
                    continue;
                }
            }
        }
    }
    Ok(())
}

/// Open (but don't start) a capture stream for `kind`.
///
/// For system audio we prefer **process loopback** (`exclude-self`): it taps every
/// other process's render stream *upstream of the endpoint mixer*, so it captures
/// the meeting no matter which output device the audio is routed to, keeps recording
/// even when the endpoint is muted, and never records our own sounds. Classic
/// **endpoint (device) loopback** is the fallback for when process loopback can't
/// activate (older Windows / an activation error): it records only the mix of one
/// specific render endpoint. On that fallback we try the **communications** default
/// render device first (where native conferencing apps route call audio), then the
/// **console** default. The supervisor re-runs this selection on a device change.
unsafe fn open_stream(kind: StreamKind) -> Result<CaptureStream, Box<dyn std::error::Error>> {
    match kind {
        StreamKind::System => {
            match setup_process_loopback() {
                Ok(s) => {
                    tracing::info!("system audio: WASAPI process loopback (exclude-self)");
                    return Ok(s);
                }
                Err(e) => tracing::warn!(
                    "system audio: process loopback unavailable ({e}); falling back to \
                     endpoint loopback (device-bound, mute-sensitive)"
                ),
            }
            match setup_endpoint_loopback(eCommunications) {
                Ok(s) => {
                    tracing::info!(
                        "system audio: endpoint loopback (communications render device)"
                    );
                    return Ok(s);
                }
                Err(e) => tracing::warn!(
                    "system audio: communications-endpoint loopback unavailable ({e}); trying console endpoint"
                ),
            }
            let s = setup_endpoint_loopback(eConsole)?;
            tracing::info!("system audio: endpoint loopback (console render device)");
            Ok(s)
        }
        StreamKind::Microphone => {
            tracing::info!("microphone: WASAPI capture endpoint");
            setup_capture_endpoint()
        }
    }
}

/// Sleep up to `dur`, waking early if `stop` is signaled. Returns `true` if a
/// stop was requested (so the caller should break out of its supervise loop).
fn sleep_unless_stopped(stop: &AtomicBool, dur: Duration) -> bool {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.load(Ordering::Relaxed)
}

/// An initialized WASAPI capture stream plus what we need to decode it.
struct CaptureStream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    /// Some(event) → event-driven (process loopback); None → polled.
    event: Option<HANDLE>,
    channels: u16,
    bytes_per_sample: usize,
    is_float: bool,
    sample_rate: u32,
    /// Which logical source this stream feeds (mic vs system).
    source: AudioSource,
    /// True when the mic was opened in exclusive mode (bypasses the shared audio
    /// engine, so it works on machines with a broken shared-mode enhancement — but
    /// seizes the device, so other apps may lose it). Always false for loopback.
    exclusive: bool,
    /// True when this is system capture on the classic endpoint-loopback fallback
    /// (device-bound, mute-sensitive) rather than process loopback. Always false
    /// for the mic and for process loopback.
    endpoint_fallback: bool,
}

/// What a started stream reports back to the caller.
#[derive(Clone, Copy)]
struct CaptureInfo {
    /// The stream is a microphone opened in exclusive mode (see `CaptureStream`).
    exclusive: bool,
    /// System capture is on the endpoint-loopback fallback (see `CaptureStream`).
    endpoint_fallback: bool,
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        if let Some(h) = self.event.take() {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Why [`pump`] returned — so the supervisor knows whether to finish or re-open.
enum PumpOutcome {
    /// Recording is over (stop requested, or the pipeline queue closed).
    Stopped,
    /// The device backing this stream went away (unplugged / default switched /
    /// audio service bounced). The supervisor should re-open on the new default.
    DeviceChanged,
}

/// True for the WASAPI errors that mean "this endpoint is gone, re-open on the
/// current default" rather than "give up". Device-invalidated fires when the
/// output device is switched or unplugged mid-capture; service-not-running fires
/// while the Windows Audio service restarts (it comes back).
fn is_device_lost(hr: windows::core::HRESULT) -> bool {
    const DEVICE_INVALIDATED: windows::core::HRESULT =
        windows::core::HRESULT(0x8889_0004u32 as i32);
    const SERVICE_NOT_RUNNING: windows::core::HRESULT =
        windows::core::HRESULT(0x8889_0010u32 as i32);
    hr == DEVICE_INVALIDATED || hr == SERVICE_NOT_RUNNING
}

/// Pull audio until stopped or the device is lost, sending each buffer as a
/// timestamped packet. Returns why it stopped so the supervisor can re-open the
/// capture on the current default device when the device changed underneath us.
unsafe fn pump(stream: &CaptureStream, stop: &AtomicBool, tx: &Sender<AudioPacket>) -> PumpOutcome {
    let silent_flag = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
    let frame_bytes = stream.channels as usize * stream.bytes_per_sample;

    while !stop.load(Ordering::Relaxed) {
        match stream.event {
            Some(h) => {
                let _ = WaitForSingleObject(h, 200);
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }

        loop {
            let packet_frames = match stream.capture.GetNextPacketSize() {
                Ok(p) => p,
                Err(e) => return classify_pump_error(e),
            };
            if packet_frames == 0 {
                break;
            }

            let mut pdata: *mut u8 = ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if let Err(e) = stream
                .capture
                .GetBuffer(&mut pdata, &mut frames, &mut flags, None, None)
            {
                return classify_pump_error(e);
            }

            if frames > 0 && !pdata.is_null() && frame_bytes > 0 {
                // Timestamp the instant the buffer is in hand.
                let timestamp = Instant::now();
                let samples = if flags & silent_flag != 0 {
                    vec![0.0f32; frames as usize * stream.channels as usize]
                } else {
                    let bytes = std::slice::from_raw_parts(pdata, frames as usize * frame_bytes);
                    decode_interleaved(bytes, stream.bytes_per_sample, stream.is_float)
                };
                let send = tx.send(AudioPacket {
                    timestamp,
                    sample_rate: stream.sample_rate,
                    channels: stream.channels,
                    samples,
                    source: stream.source,
                });
                if send.is_err() {
                    let _ = stream.capture.ReleaseBuffer(frames);
                    return PumpOutcome::Stopped; // queue closed → recording stopped
                }
            }

            let _ = stream.capture.ReleaseBuffer(frames);
        }
    }
    PumpOutcome::Stopped
}

/// Map a capture-read error to a pump outcome: re-open on a lost device, stop on
/// anything else (better than the old behavior of spinning on a dead endpoint).
fn classify_pump_error(e: windows::core::Error) -> PumpOutcome {
    if is_device_lost(e.code()) {
        PumpOutcome::DeviceChanged
    } else {
        tracing::warn!("system audio: capture read error ({e}); stopping this stream");
        PumpOutcome::Stopped
    }
}

/// Pull packets for up to `timeout`, forwarding any read, and return `true` as
/// soon as one real (non-empty) buffer arrives. Used to confirm a microphone
/// endpoint truly delivers PCM before we commit to it — a silent buffer still
/// counts (the device is producing frames), so a quiet or muted mic is not a
/// false negative; only a stream that delivers *nothing* fails.
unsafe fn validate_capture(
    stream: &CaptureStream,
    tx: &Sender<AudioPacket>,
    timeout: Duration,
) -> bool {
    let silent_flag = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
    let frame_bytes = stream.channels as usize * stream.bytes_per_sample;
    if frame_bytes == 0 {
        return false;
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match stream.event {
            Some(h) => {
                let _ = WaitForSingleObject(h, 100);
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
        loop {
            let packet_frames = match stream.capture.GetNextPacketSize() {
                Ok(p) => p,
                Err(_) => return false,
            };
            if packet_frames == 0 {
                break;
            }
            let mut pdata: *mut u8 = ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if stream
                .capture
                .GetBuffer(&mut pdata, &mut frames, &mut flags, None, None)
                .is_err()
            {
                return false;
            }
            let delivered = frames > 0 && !pdata.is_null();
            if delivered {
                let timestamp = Instant::now();
                let samples = if flags & silent_flag != 0 {
                    vec![0.0f32; frames as usize * stream.channels as usize]
                } else {
                    let bytes = std::slice::from_raw_parts(pdata, frames as usize * frame_bytes);
                    decode_interleaved(bytes, stream.bytes_per_sample, stream.is_float)
                };
                let _ = tx.send(AudioPacket {
                    timestamp,
                    sample_rate: stream.sample_rate,
                    channels: stream.channels,
                    samples,
                    source: stream.source,
                });
            }
            let _ = stream.capture.ReleaseBuffer(frames);
            if delivered {
                return true;
            }
        }
    }
    false
}

// ------------------------------------------------------------- process loopback

/// COM object that signals a condvar when `ActivateAudioInterfaceAsync` finishes.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    signal: Arc<(Mutex<bool>, Condvar)>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _op: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let (lock, cvar) = &*self.signal;
        if let Ok(mut done) = lock.lock() {
            *done = true;
        }
        cvar.notify_all();
        Ok(())
    }
}

/// A hand-built `PROPVARIANT` holding a BLOB (x64 C layout).
#[repr(C)]
struct PropVariantBlob {
    vt: u16,
    reserved1: u16,
    reserved2: u16,
    reserved3: u16,
    cb_size: u32,
    _pad: u32,
    blob_data: *mut c_void,
}

unsafe fn setup_process_loopback() -> Result<CaptureStream, Box<dyn std::error::Error>> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams.TargetProcessId = std::process::id();
    params.Anonymous.ProcessLoopbackParams.ProcessLoopbackMode =
        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE;

    let blob = PropVariantBlob {
        vt: VT_BLOB,
        reserved1: 0,
        reserved2: 0,
        reserved3: 0,
        cb_size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        _pad: 0,
        blob_data: &mut params as *mut _ as *mut c_void,
    };
    let propvariant = &blob as *const PropVariantBlob as *const windows::core::PROPVARIANT;

    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        signal: signal.clone(),
    }
    .into();

    let op: IActivateAudioInterfaceAsyncOperation = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &<IAudioClient as Interface>::IID,
        Some(propvariant),
        &handler,
    )?;

    {
        let (lock, cvar) = &*signal;
        let mut done = lock.lock().unwrap();
        while !*done {
            let (guard, timeout) = cvar
                .wait_timeout(done, Duration::from_secs(5))
                .map_err(|_| "activation condvar poisoned")?;
            done = guard;
            if timeout.timed_out() && !*done {
                return Err("process-loopback activation timed out".into());
            }
        }
    }

    let mut hr = windows::core::HRESULT(0);
    let mut unknown: Option<IUnknown> = None;
    op.GetActivateResult(&mut hr, &mut unknown)?;
    hr.ok()?;
    let client: IAudioClient = unknown.ok_or("activation returned no interface")?.cast()?;

    let format = pcm_format(CAPTURE_RATE, 2, 16);
    client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        0,
        0,
        &format,
        None,
    )?;

    let event = CreateEventW(None, false, false, PCWSTR::null())?;
    if let Err(e) = client.SetEventHandle(event) {
        let _ = CloseHandle(event);
        return Err(e.into());
    }
    let capture: IAudioCaptureClient = match client.GetService() {
        Ok(c) => c,
        Err(e) => {
            let _ = CloseHandle(event);
            return Err(e.into());
        }
    };

    Ok(CaptureStream {
        client,
        capture,
        event: Some(event),
        channels: 2,
        bytes_per_sample: 2,
        is_float: false,
        sample_rate: CAPTURE_RATE,
        source: AudioSource::System,
        exclusive: false,
        endpoint_fallback: false,
    })
}

// ------------------------------------------------------------ endpoint loopback

unsafe fn setup_endpoint_loopback(role: ERole) -> Result<CaptureStream, Box<dyn std::error::Error>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| format!("CoCreateInstance: {e}"))?;
    let device: IMMDevice = enumerator
        .GetDefaultAudioEndpoint(eRender, role)
        .map_err(|e| format!("GetDefaultAudioEndpoint: {e}"))?;
    let client: IAudioClient = device
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| format!("Activate: {e}"))?;

    let pformat = client.GetMixFormat().map_err(|e| format!("GetMixFormat: {e}"))?;
    let wfx = ptr::read_unaligned(pformat);
    let channels = wfx.nChannels;
    let sample_rate = wfx.nSamplesPerSec;
    let block_align = wfx.nBlockAlign as usize;
    let format_tag = wfx.wFormatTag;
    let bytes_per_sample = if channels > 0 {
        block_align / channels as usize
    } else {
        0
    };
    let is_float = match format_tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_EXTENSIBLE => {
            let ext = ptr::read_unaligned(pformat as *const WAVEFORMATEXTENSIBLE);
            let sub = ext.SubFormat;
            sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        }
        _ => false,
    };
    if bytes_per_sample == 0 {
        CoTaskMemFree(Some(pformat as *const c_void));
        return Err("endpoint loopback: invalid mix format".into());
    }

    let init = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        0,
        0,
        pformat,
        None,
    );
    CoTaskMemFree(Some(pformat as *const c_void));
    init.map_err(|e| format!("Initialize: {e}"))?;

    let capture: IAudioCaptureClient = client.GetService()?;

    Ok(CaptureStream {
        client,
        capture,
        event: None,
        channels,
        bytes_per_sample,
        is_float,
        sample_rate,
        source: AudioSource::System,
        exclusive: false,
        endpoint_fallback: true,
    })
}

// ------------------------------------------------------------- capture endpoint

/// Open the default microphone. Tries shared mode first (the normal case), then
/// falls back to **exclusive mode** — which bypasses the shared audio engine and
/// succeeds on machines whose shared-mode path rejects `Initialize` with
/// E_INVALIDARG.
unsafe fn setup_capture_endpoint() -> Result<CaptureStream, Box<dyn std::error::Error>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| format!("CoCreateInstance: {e}"))?;
    let device: IMMDevice = enumerator
        .GetDefaultAudioEndpoint(eCapture, eCommunications)
        .map_err(|e| format!("GetDefaultAudioEndpoint(capture): {e}"))?;

    match setup_capture_shared(&device) {
        Ok(s) => {
            tracing::info!("microphone: shared-mode capture");
            Ok(s)
        }
        Err(e) => {
            tracing::warn!("microphone: shared-mode failed ({e}); trying exclusive mode");
            let s = setup_capture_exclusive(&device)?;
            tracing::info!("microphone: exclusive-mode capture");
            Ok(s)
        }
    }
}

/// Shared-mode capture using the device mix format.
unsafe fn setup_capture_shared(device: &IMMDevice) -> Result<CaptureStream, Box<dyn std::error::Error>> {
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
    let pformat = client.GetMixFormat().map_err(|e| format!("GetMixFormat: {e}"))?;
    let wfx = ptr::read_unaligned(pformat);
    let channels = wfx.nChannels;
    let sample_rate = wfx.nSamplesPerSec;
    let block_align = wfx.nBlockAlign as usize;
    let format_tag = wfx.wFormatTag;
    let bytes_per_sample = if channels > 0 { block_align / channels as usize } else { 0 };
    let is_float = match format_tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_EXTENSIBLE => {
            let ext = ptr::read_unaligned(pformat as *const WAVEFORMATEXTENSIBLE);
            let sub = ext.SubFormat;
            sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        }
        _ => false,
    };
    if bytes_per_sample == 0 {
        CoTaskMemFree(Some(pformat as *const c_void));
        return Err("capture endpoint: invalid mix format".into());
    }
    let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, pformat, None);
    CoTaskMemFree(Some(pformat as *const c_void));
    init.map_err(|e| format!("shared Initialize: {e}"))?;
    let capture: IAudioCaptureClient = client.GetService()?;
    Ok(CaptureStream {
        client,
        capture,
        event: None,
        channels,
        bytes_per_sample,
        is_float,
        sample_rate,
        source: AudioSource::Microphone,
        exclusive: false,
        endpoint_fallback: false,
    })
}

/// Exclusive-mode capture. Exclusive mode bypasses the shared audio engine (and
/// any broken enhancement/APO living in it), so it succeeds on machines whose
/// shared-mode `Initialize` returns E_INVALIDARG — but it requires a format the
/// hardware *natively* supports, so we negotiate across the device's native format
/// and common PCM formats rather than assuming 48 kHz stereo (which is why the old
/// hard-coded format only worked on machines whose mic happened to match it).
/// Event-driven, with the mandatory buffer-alignment retry per candidate.
unsafe fn setup_capture_exclusive(device: &IMMDevice) -> Result<CaptureStream, Box<dyn std::error::Error>> {
    // AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED — the format is accepted, only the buffer
    // size needs re-aligning; retry once with the size the engine reports.
    const NOT_ALIGNED: windows::core::HRESULT = windows::core::HRESULT(0x8889_0019u32 as i32);
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;

    // The device's native (mix) rate/channels is the format most likely to be
    // accepted in exclusive mode; read it so we try it first.
    let (native_rate, native_ch) = {
        let probe: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
        match probe.GetMixFormat() {
            Ok(p) => {
                let w = ptr::read_unaligned(p);
                let out = (w.nSamplesPerSec, w.nChannels.max(1));
                CoTaskMemFree(Some(p as *const c_void));
                out
            }
            Err(_) => (CAPTURE_RATE, 2),
        }
    };

    // Candidate PCM formats, most-preferred first: native rate/channels, then
    // common meeting formats. De-duplicated so we probe each format at most once.
    let mut candidates: Vec<(u32, u16, u16)> = Vec::new();
    for &rate in &[native_rate, 48_000, 44_100, 16_000] {
        for &ch in &[native_ch, 2, 1] {
            for &bits in &[16u16, 24, 32] {
                let cand = (rate, ch, bits);
                if !candidates.contains(&cand) {
                    candidates.push(cand);
                }
            }
        }
    }

    // Buffer duration = the device minimum period (re-aligned per candidate below).
    let min_period = {
        let probe: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
        let mut default_period = 0i64;
        let mut min_period = 0i64;
        probe
            .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))
            .map_err(|e| format!("GetDevicePeriod: {e}"))?;
        if min_period <= 0 { 30_000 } else { min_period }
    };

    let mut last_err = String::from("no exclusive-mode format was accepted");
    for (rate, channels, bits) in candidates {
        let format = pcm_format(rate, channels, bits);
        let mut client: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("Activate: {e}");
                continue;
            }
        };
        let mut dur = min_period;
        let mut init = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, flags, dur, dur, &format, None);
        if let Err(e) = &init {
            if e.code() == NOT_ALIGNED {
                if let Ok(frames) = client.GetBufferSize() {
                    // Aligned duration (hns) for the reported buffer frame count.
                    dur = ((10_000.0 * 1_000.0 / rate as f64) * frames as f64 + 0.5) as i64;
                    if let Ok(c) = device.Activate(CLSCTX_ALL, None) {
                        client = c;
                        init = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, flags, dur, dur, &format, None);
                    }
                }
            }
        }
        if let Err(e) = init {
            last_err = format!("{rate}Hz/{channels}ch/{bits}bit: {e}");
            continue;
        }

        let event = CreateEventW(None, false, false, PCWSTR::null())?;
        if let Err(e) = client.SetEventHandle(event) {
            let _ = CloseHandle(event);
            return Err(e.into());
        }
        let capture: IAudioCaptureClient = match client.GetService() {
            Ok(c) => c,
            Err(e) => {
                let _ = CloseHandle(event);
                return Err(e.into());
            }
        };
        tracing::info!("microphone: exclusive-mode format {rate}Hz/{channels}ch/{bits}bit");
        return Ok(CaptureStream {
            client,
            capture,
            event: Some(event),
            channels,
            bytes_per_sample: (bits / 8) as usize,
            is_float: false,
            sample_rate: rate,
            source: AudioSource::Microphone,
            exclusive: true,
            endpoint_fallback: false,
        });
    }
    Err(last_err.into())
}

// -------------------------------------------------------------------- decoding

/// Build a simple integer-PCM `WAVEFORMATEX`.
fn pcm_format(rate: u32, channels: u16, bits: u16) -> WAVEFORMATEX {
    let block_align = channels * (bits / 8);
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM,
        nChannels: channels,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * block_align as u32,
        nBlockAlign: block_align,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

/// Decode an interleaved native-format buffer to interleaved f32 (channels kept).
fn decode_interleaved(bytes: &[u8], bytes_per_sample: usize, is_float: bool) -> Vec<f32> {
    if bytes_per_sample == 0 {
        return Vec::new();
    }
    bytes
        .chunks_exact(bytes_per_sample)
        .map(|s| decode_sample(s, is_float))
        .collect()
}

/// Decode one native-format sample to f32 in [-1, 1].
fn decode_sample(bytes: &[u8], is_float: bool) -> f32 {
    match (is_float, bytes.len()) {
        (true, 4) => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        (true, 8) => f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]) as f32,
        (false, 2) => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        (false, 3) => {
            let v = ((bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16)) << 8;
            (v >> 8) as f32 / 8_388_608.0
        }
        (false, 4) => {
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 2_147_483_648.0
        }
        _ => 0.0,
    }
}

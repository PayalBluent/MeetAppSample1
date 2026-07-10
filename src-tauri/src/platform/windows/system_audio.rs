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
    eCapture, eCommunications, eConsole, eRender, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
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
/// to `tx`. Blocks until the stream is running; returns the thread handle or `None`.
pub fn start(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    start_with(tx, stop, StreamKind::System, "meetapp-system-audio")
}

/// Start microphone capture through a raw WASAPI capture endpoint. This is the
/// fallback the mic uses when cpal can't open the default input device.
pub fn start_microphone(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    start_with(tx, stop, StreamKind::Microphone, "meetapp-wasapi-mic")
}

fn start_with(
    tx: Sender<AudioPacket>,
    stop: Arc<AtomicBool>,
    kind: StreamKind,
    thread_name: &str,
) -> Option<JoinHandle<()>> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let join = std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            if let Err(e) = run(&stop, &tx, &ready_tx, kind) {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        })
        .ok()?;

    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Some(join),
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
    ready: &mpsc::Sender<Result<(), String>>,
    kind: StreamKind,
) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let stream = match kind {
            StreamKind::System => match setup_process_loopback() {
                Ok(s) => {
                    tracing::info!("system audio: WASAPI process loopback (exclude-self)");
                    s
                }
                Err(e) => {
                    tracing::warn!("process loopback unavailable ({e}); trying endpoint loopback");
                    setup_endpoint_loopback()?
                }
            },
            StreamKind::Microphone => {
                tracing::info!("microphone: WASAPI capture endpoint");
                setup_capture_endpoint()?
            }
        };

        stream.client.Start()?;
        let _ = ready.send(Ok(()));

        pump(&stream, stop, tx);

        let _ = stream.client.Stop();
        // `stream` drops here → its Drop closes the event handle.
    }
    Ok(())
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

/// Pull audio until stopped, sending each buffer as a timestamped packet.
unsafe fn pump(stream: &CaptureStream, stop: &AtomicBool, tx: &Sender<AudioPacket>) {
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
                Err(_) => break,
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
                break;
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
                    return; // queue closed → recording stopped
                }
            }

            let _ = stream.capture.ReleaseBuffer(frames);
        }
    }
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
    })
}

// ------------------------------------------------------------ endpoint loopback

unsafe fn setup_endpoint_loopback() -> Result<CaptureStream, Box<dyn std::error::Error>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| format!("CoCreateInstance: {e}"))?;
    let device: IMMDevice = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
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
    })
}

/// Exclusive-mode capture with a hand-built 16-bit PCM format. Event-driven, with
/// the mandatory buffer-alignment retry.
unsafe fn setup_capture_exclusive(device: &IMMDevice) -> Result<CaptureStream, Box<dyn std::error::Error>> {
    // AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED — the format is accepted, only the buffer
    // size needs re-aligning; retry once with the size the engine reports.
    const NOT_ALIGNED: windows::core::HRESULT = windows::core::HRESULT(0x8889_0019u32 as i32);
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    let format = pcm_format(CAPTURE_RATE, 2, 16);

    // Buffer duration = the device minimum period.
    let mut default_period = 0i64;
    let mut min_period = 0i64;
    {
        let probe: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
        probe
            .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))
            .map_err(|e| format!("GetDevicePeriod: {e}"))?;
    }
    if min_period <= 0 {
        min_period = 30_000; // 3 ms fallback
    }

    let mut client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
    let mut dur = min_period;
    let mut init = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, flags, dur, dur, &format, None);

    if let Err(e) = &init {
        if e.code() == NOT_ALIGNED {
            let frames = client.GetBufferSize().map_err(|e| format!("GetBufferSize: {e}"))?;
            // Aligned duration (hns) for the reported buffer frame count.
            dur = ((10_000.0 * 1_000.0 / CAPTURE_RATE as f64) * frames as f64 + 0.5) as i64;
            client = device.Activate(CLSCTX_ALL, None).map_err(|e| format!("Activate: {e}"))?;
            init = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, flags, dur, dur, &format, None);
        }
    }
    init.map_err(|e| format!("exclusive Initialize: {e}"))?;

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
        source: AudioSource::Microphone,
    })
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

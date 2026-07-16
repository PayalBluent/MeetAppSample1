//! Audio capture diagnostic — run with:
//!   cargo run --example audio_diag
//!
//! Enumerates and PROBES every microphone (cpal) and every WASAPI endpoint,
//! actually opening each stream and validating that PCM callbacks deliver
//! samples. For every device it prints: the negotiated format, whether the
//! stream opened, whether the callback fired, how many samples arrived, and the
//! peak amplitude (so you can tell "opened but silent" from "opened with audio").
//! Every failure prints the exact error and HRESULT.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    println!("=== MeetApp audio diagnostic ===\n");
    #[cfg(windows)]
    session_info();
    probe_cpal();
    #[cfg(windows)]
    {
        println!();
        probe_wasapi();
    }
    println!("\n=== done ===");
}

// ------------------------------------------------------------------- cpal (mic)

fn bump_peak(peak: &AtomicU32, v: f32) {
    let cur = f32::from_bits(peak.load(Ordering::Relaxed));
    if v > cur {
        peak.store(v.to_bits(), Ordering::Relaxed);
    }
}

fn err_cb(e: cpal::StreamError) {
    println!("      STREAM ERROR (async): {e}");
}

fn probe_cpal() {
    use cpal::traits::{DeviceTrait, HostTrait};

    println!("--- cpal microphone probe ---");
    println!("available hosts: {:?}", cpal::available_hosts());

    let host = cpal::default_host();
    println!("default host: {:?}", host.id());

    match host.default_input_device() {
        Some(d) => println!(
            "default input device: {:?}",
            d.name().unwrap_or_else(|_| "<name error>".into())
        ),
        None => println!("default input device: NONE"),
    }

    let devices: Vec<cpal::Device> = match host.input_devices() {
        Ok(it) => it.collect(),
        Err(e) => {
            println!("input_devices() ERROR: {e}");
            Vec::new()
        }
    };
    println!("enumerated {} input device(s)\n", devices.len());

    for (i, device) in devices.iter().enumerate() {
        let name = device.name().unwrap_or_else(|_| "<name error>".into());
        println!("[{i}] {name}");

        match device.default_input_config() {
            Ok(cfg) => println!(
                "    default config: {:?}, {} ch @ {} Hz",
                cfg.sample_format(),
                cfg.channels(),
                cfg.sample_rate().0
            ),
            Err(e) => println!("    default_input_config ERROR: {e}"),
        }

        match device.supported_input_configs() {
            Ok(cfgs) => {
                for c in cfgs {
                    println!(
                        "    supported: {:?}, {} ch, {}..{} Hz",
                        c.sample_format(),
                        c.channels(),
                        c.min_sample_rate().0,
                        c.max_sample_rate().0
                    );
                }
            }
            Err(e) => println!("    supported_input_configs ERROR: {e}"),
        }

        probe_cpal_open(device);
        println!();
    }
}

/// Try the device's default config, then every supported config, and report per
/// config whether the stream opened and delivered PCM (mirrors what the app now
/// does). Stops after the first config that actually delivers samples.
fn probe_cpal_open(device: &cpal::Device) {
    use cpal::traits::DeviceTrait;

    let mut configs: Vec<(cpal::SampleFormat, cpal::StreamConfig)> = Vec::new();
    if let Ok(def) = device.default_input_config() {
        configs.push((def.sample_format(), def.into()));
    }
    if let Ok(ranges) = device.supported_input_configs() {
        for r in ranges {
            let sc = r.with_max_sample_rate();
            let fmt = sc.sample_format();
            let cfg: cpal::StreamConfig = sc.into();
            if !configs
                .iter()
                .any(|(f, c)| *f == fmt && c.channels == cfg.channels && c.sample_rate == cfg.sample_rate)
            {
                configs.push((fmt, cfg));
            }
        }
    }
    if configs.is_empty() {
        println!("    OPEN: no configs to try");
        return;
    }
    for (fmt, config) in configs {
        if probe_cpal_config(device, fmt, &config) {
            return; // found a working format for this device
        }
    }
}

/// Returns true if this config opened AND delivered PCM samples.
fn probe_cpal_config(device: &cpal::Device, fmt: cpal::SampleFormat, config: &cpal::StreamConfig) -> bool {
    use cpal::traits::{DeviceTrait, StreamTrait};

    print!(
        "    OPEN {:?} {}ch @ {}Hz: ",
        fmt, config.channels, config.sample_rate.0
    );
    let count = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let c = count.clone();
    let p = peak.clone();

    macro_rules! stream {
        ($t:ty, $conv:expr) => {
            device.build_input_stream(
                config,
                move |data: &[$t], _| {
                    c.fetch_add(data.len(), Ordering::Relaxed);
                    let m = data.iter().fold(0.0f32, |a, &x| a.max(($conv)(x).abs()));
                    bump_peak(&p, m);
                },
                err_cb,
                None,
            )
        };
    }
    let build = match fmt {
        cpal::SampleFormat::F32 => stream!(f32, |x: f32| x),
        cpal::SampleFormat::F64 => stream!(f64, |x: f64| x as f32),
        cpal::SampleFormat::I8 => stream!(i8, |x: i8| x as f32 / 128.0),
        cpal::SampleFormat::U8 => stream!(u8, |x: u8| (x as f32 / 128.0) - 1.0),
        cpal::SampleFormat::I16 => stream!(i16, |x: i16| x as f32 / 32768.0),
        cpal::SampleFormat::U16 => stream!(u16, |x: u16| (x as f32 / 32768.0) - 1.0),
        cpal::SampleFormat::I32 => stream!(i32, |x: i32| x as f32 / 2_147_483_648.0),
        other => {
            println!("unsupported sample format {other:?}");
            return false;
        }
    };

    let stream = match build {
        Ok(s) => s,
        Err(e) => {
            println!("build FAILED: {e}");
            return false;
        }
    };
    if let Err(e) = stream.play() {
        println!("play FAILED: {e}");
        return false;
    }
    std::thread::sleep(Duration::from_millis(600));
    let n = count.load(Ordering::Relaxed);
    let pk = f32::from_bits(peak.load(Ordering::Relaxed));
    drop(stream);

    if n == 0 {
        println!("opened but NO PCM (callback never fired)");
        false
    } else if pk == 0.0 {
        println!("OK opened, {n} samples, but SILENT (make noise / unmute) — WORKS");
        true
    } else {
        println!("OK WORKING — {n} samples, peak {pk:.4} (non-zero audio)");
        true
    }
}

// ---------------------------------------------------------------- WASAPI probe

/// Is this an RDP / remote / virtualized session? That routinely makes endpoints
/// enumerable but un-openable (shared-mode Initialize -> E_INVALIDARG).
#[cfg(windows)]
fn session_info() {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
    let remote = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
    let session = std::env::var("SESSIONNAME").unwrap_or_default();
    println!(
        "session: remote/RDP={} SESSIONNAME={:?}",
        remote,
        if session.is_empty() { "<unset>".into() } else { session }
    );
    println!();
}

#[cfg(windows)]
fn probe_wasapi() {
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    println!("--- WASAPI endpoint probe ---");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            Ok(e) => e,
            Err(e) => {
                println!("CoCreateInstance(MMDeviceEnumerator) FAILED: {e}");
                return;
            }
        };

        probe_wasapi_flow(&en, eCapture, "CAPTURE (microphones)", false);
        probe_wasapi_flow(&en, eRender, "RENDER (loopback sources)", true);

        // The decisive experiment: which Initialize STRATEGY actually works?
        println!("\n  === Initialize strategy matrix: default CAPTURE endpoint (mic) ===");
        match en.GetDefaultAudioEndpoint(eCapture, eCommunications) {
            Ok(dev) => {
                probe_init_strategies(&dev, false);
                probe_exclusive_and_support(&dev, "mic");
            }
            Err(e) => println!("    GetDefaultAudioEndpoint(eCapture) FAILED: {e}"),
        }
        println!("\n  === Initialize strategy matrix: default RENDER endpoint (loopback) ===");
        match en.GetDefaultAudioEndpoint(eRender, eConsole) {
            Ok(dev) => probe_init_strategies(&dev, true),
            Err(e) => println!("    GetDefaultAudioEndpoint(eRender) FAILED: {e}"),
        }

        // The decisive test for SYSTEM AUDIO: the app captures system output via
        // *process loopback* (a virtual device), NOT endpoint loopback. Nothing
        // above exercises that path, so probe it directly here.
        println!("\n  === Process loopback (the path the app uses for system audio) ===");
        probe_process_loopback();
    }
}

/// Activate and pump the process-loopback virtual device exactly as the app does
/// (`ActivateAudioInterfaceAsync` + `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK`,
/// exclude-self), for ~3 s. **Play audio (music / a video) while this runs.**
/// Prints whether activation and Initialize succeeded and how many packets / what
/// peak amplitude arrived — i.e. "activated but silent" vs "capturing real audio".
#[cfg(windows)]
fn probe_process_loopback() {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use windows::core::{implement, Interface, IUnknown, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Media::Audio::{
        ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
        IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
        IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        WAVEFORMATEX,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const VT_BLOB: u16 = 65;

    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Handler {
        signal: Arc<(Mutex<bool>, Condvar)>,
    }
    impl IActivateAudioInterfaceCompletionHandler_Impl for Handler_Impl {
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

    #[repr(C)]
    struct PropVariantBlob {
        vt: u16,
        r1: u16,
        r2: u16,
        r3: u16,
        cb_size: u32,
        _pad: u32,
        blob_data: *mut c_void,
    }

    unsafe {
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
        params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
        params.Anonymous.ProcessLoopbackParams.TargetProcessId = std::process::id();
        params.Anonymous.ProcessLoopbackParams.ProcessLoopbackMode =
            PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE;

        let blob = PropVariantBlob {
            vt: VT_BLOB,
            r1: 0,
            r2: 0,
            r3: 0,
            cb_size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            _pad: 0,
            blob_data: &mut params as *mut _ as *mut c_void,
        };
        let propvariant = &blob as *const PropVariantBlob as *const windows::core::PROPVARIANT;

        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let handler: IActivateAudioInterfaceCompletionHandler = Handler {
            signal: signal.clone(),
        }
        .into();

        let op: IActivateAudioInterfaceAsyncOperation = match ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &<IAudioClient as Interface>::IID,
            Some(propvariant),
            &handler,
        ) {
            Ok(o) => o,
            Err(e) => {
                println!("    ActivateAudioInterfaceAsync FAILED: {e} [0x{:08X}]", e.code().0 as u32);
                println!("    -> process loopback is UNAVAILABLE (older Windows?); app falls back to endpoint loopback");
                return;
            }
        };

        {
            let (lock, cvar) = &*signal;
            let mut done = lock.lock().unwrap();
            let mut waited = 0;
            while !*done && waited < 50 {
                let (g, _) = cvar.wait_timeout(done, Duration::from_millis(100)).unwrap();
                done = g;
                waited += 1;
            }
            if !*done {
                println!("    activation callback never completed (timeout)");
                return;
            }
        }

        let mut hr = windows::core::HRESULT(0);
        let mut unknown: Option<IUnknown> = None;
        if let Err(e) = op.GetActivateResult(&mut hr, &mut unknown) {
            println!("    GetActivateResult FAILED: {e}");
            return;
        }
        if let Err(e) = hr.ok() {
            println!("    activation result HRESULT FAILED: {e} [0x{:08X}]", hr.0 as u32);
            return;
        }
        let client: IAudioClient = match unknown.and_then(|u| u.cast().ok()) {
            Some(c) => c,
            None => {
                println!("    activation returned no IAudioClient");
                return;
            }
        };
        println!("    ActivateAudioInterfaceAsync OK (activated)");

        let format = pcm_format(48_000, 2, 16);
        if let Err(e) = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            0,
            0,
            &format as *const WAVEFORMATEX,
            None,
        ) {
            println!("    Initialize FAILED: {e} [0x{:08X}]", e.code().0 as u32);
            return;
        }
        println!("    Initialize(LOOPBACK|EVENTCALLBACK, 48k/2ch/16) OK");

        let event = match CreateEventW(None, false, false, PCWSTR::null()) {
            Ok(h) => h,
            Err(e) => {
                println!("    CreateEventW FAILED: {e}");
                return;
            }
        };
        if let Err(e) = client.SetEventHandle(event) {
            println!("    SetEventHandle FAILED: {e}");
            let _ = CloseHandle(event);
            return;
        }
        let capture: IAudioCaptureClient = match client.GetService() {
            Ok(c) => c,
            Err(e) => {
                println!("    GetService(IAudioCaptureClient) FAILED: {e}");
                let _ = CloseHandle(event);
                return;
            }
        };
        if let Err(e) = client.Start() {
            println!("    Start FAILED: {e}");
            let _ = CloseHandle(event);
            return;
        }

        println!("    capturing 3 s — PLAY SOME AUDIO NOW…");
        let silent_flag = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
        let mut packets = 0u64;
        let mut peak = 0.0f32;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let _ = WaitForSingleObject(event, 200);
            loop {
                let n = match capture.GetNextPacketSize() {
                    Ok(n) => n,
                    Err(e) => {
                        println!("    GetNextPacketSize error: {e}");
                        break;
                    }
                };
                if n == 0 {
                    break;
                }
                let mut pdata: *mut u8 = ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                if capture
                    .GetBuffer(&mut pdata, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }
                if frames > 0 && !pdata.is_null() && flags & silent_flag == 0 {
                    let samples =
                        std::slice::from_raw_parts(pdata as *const i16, frames as usize * 2);
                    for &s in samples {
                        peak = peak.max((s as f32 / 32768.0).abs());
                    }
                }
                packets += 1;
                let _ = capture.ReleaseBuffer(frames);
            }
        }
        let _ = client.Stop();
        let _ = CloseHandle(event);

        if packets == 0 {
            println!("    RESULT: activated but delivered NO PACKETS (nothing was playing, or the audio bypasses the mixer)");
        } else if peak == 0.0 {
            println!("    RESULT: {packets} packets, but SILENT — was audio actually playing? (all buffers silent)");
        } else {
            println!("    RESULT: WORKING — {packets} packets, peak {peak:.4} (real system audio captured)");
        }
    }
}

/// Simple integer-PCM WAVEFORMATEX (matches the format the process-loopback path
/// hand-builds — the one that already Initializes successfully here).
#[cfg(windows)]
fn pcm_format(rate: u32, channels: u16, bits: u16) -> windows::Win32::Media::Audio::WAVEFORMATEX {
    let block_align = channels * (bits / 8);
    windows::Win32::Media::Audio::WAVEFORMATEX {
        wFormatTag: 1, // WAVE_FORMAT_PCM
        nChannels: channels,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * block_align as u32,
        nBlockAlign: block_align,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

/// Try a matrix of Initialize strategies on a fresh client each time; report
/// which one succeeds. `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY` lets shared-mode
/// accept a format different from the device mix format (the engine resamples).
#[cfg(windows)]
unsafe fn probe_init_strategies(dev: &windows::Win32::Media::Audio::IMMDevice, loopback: bool) {
    use std::ptr;
    use windows::Win32::Media::Audio::{AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX};

    const AUTOCONVERTPCM: u32 = 0x8000_0000;
    const SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
    let base: u32 = if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
    let autoconv = AUTOCONVERTPCM | SRC_DEFAULT_QUALITY;

    // Read the device mix format's rate/channels for a matched-PCM attempt.
    let (mix_rate, mix_ch) = {
        use windows::Win32::Media::Audio::IAudioClient;
        use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};
        match dev.Activate::<IAudioClient>(CLSCTX_ALL, None) {
            Ok(c) => match c.GetMixFormat() {
                Ok(p) => {
                    let w: WAVEFORMATEX = ptr::read_unaligned(p);
                    let out = (w.nSamplesPerSec, w.nChannels);
                    CoTaskMemFree(Some(p as *const _));
                    out
                }
                Err(_) => (48_000, 2),
            },
            Err(_) => (48_000, 2),
        }
    };

    let pcm_s48 = pcm_format(48_000, 2, 16);
    let pcm_m48 = pcm_format(48_000, 1, 16);
    let pcm_mix = pcm_format(mix_rate, mix_ch, 16);

    let strategies: [(&str, u32, *const WAVEFORMATEX); 4] = [
        ("PCM16 stereo 48k, no-flags", base, &pcm_s48),
        ("PCM16 stereo 48k, AUTOCONVERT", base | autoconv, &pcm_s48),
        ("PCM16 mono 48k, AUTOCONVERT", base | autoconv, &pcm_m48),
        (
            "PCM16 @mix rate/ch, AUTOCONVERT",
            base | autoconv,
            &pcm_mix,
        ),
    ];

    // Strategy 0: the device's own mix format (baseline, expected to fail here).
    try_init(dev, base, ptr::null(), 0, "mix format, buf=0");
    for (label, flags, fmt) in strategies {
        try_init(dev, flags, fmt, 0, label);
    }

    // Maybe hnsBufferDuration=0 is the rejected parameter — try the device's
    // default period and a fixed 20 ms buffer with the mix format.
    let period = device_default_period(dev);
    if period > 0 {
        try_init(dev, base, ptr::null(), period, "mix format, buf=devicePeriod");
    }
    try_init(dev, base, ptr::null(), 200_000, "mix format, buf=20ms");
}

/// Default device period (hns) via GetDevicePeriod, or 0 if unavailable.
#[cfg(windows)]
unsafe fn device_default_period(dev: &windows::Win32::Media::Audio::IMMDevice) -> i64 {
    use windows::Win32::Media::Audio::IAudioClient;
    use windows::Win32::System::Com::CLSCTX_ALL;
    let Ok(client) = dev.Activate::<IAudioClient>(CLSCTX_ALL, None) else {
        return 0;
    };
    let mut default_period = 0i64;
    let mut min_period = 0i64;
    if client
        .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))
        .is_ok()
    {
        println!("    (device period: default={default_period} hns, min={min_period} hns)");
        default_period
    } else {
        0
    }
}

/// Decisive layer test: (1) does the shared engine even *accept* a format query
/// (`IsFormatSupported`)? (2) can the hardware stream at all in EXCLUSIVE mode,
/// which bypasses the shared audio engine and any audio-enhancement APO? If
/// exclusive works but shared fails, the fault is the shared engine / an
/// enhancement APO / the driver's shared path — fixable at the system level.
#[cfg(windows)]
unsafe fn probe_exclusive_and_support(dev: &windows::Win32::Media::Audio::IMMDevice, label: &str) {
    use windows::Win32::Media::Audio::{
        AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED, IAudioClient,
    };
    use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

    // AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED — means the format IS accepted in
    // exclusive mode, just needs an aligned buffer (i.e. hardware can stream).
    const NOT_ALIGNED: i32 = 0x8889_0019u32 as i32;

    // Minimum device period, used as the exclusive-mode buffer duration.
    let min_period = {
        let mut d = 0i64;
        let mut m = 0i64;
        match dev.Activate::<IAudioClient>(CLSCTX_ALL, None) {
            Ok(c) if c.GetDevicePeriod(Some(&mut d), Some(&mut m)).is_ok() => m,
            _ => 30_000,
        }
    };

    // 1) Does the shared engine even accept a format QUERY for the mix format?
    //    NOTE: shared mode REQUIRES a non-null closest-match out-param, or the
    //    call returns E_POINTER (0x80004003) — which is NOT a device fault.
    if let Ok(c) = dev.Activate::<IAudioClient>(CLSCTX_ALL, None) {
        if let Ok(p) = c.GetMixFormat() {
            let mut closest: *mut windows::Win32::Media::Audio::WAVEFORMATEX =
                std::ptr::null_mut();
            let hr = c.IsFormatSupported(
                AUDCLNT_SHAREMODE_SHARED,
                p,
                Some(&mut closest as *mut *mut _),
            );
            println!(
                "    [{label}] IsFormatSupported(SHARED, mix) = 0x{:08X} ({})",
                hr.0 as u32,
                if hr.is_ok() { "accepted" } else { "not accepted" }
            );
            if !closest.is_null() {
                CoTaskMemFree(Some(closest as *const _));
            }
            CoTaskMemFree(Some(p as *const _));
        }
    }

    // 2) Can the raw hardware stream in EXCLUSIVE mode (bypasses shared engine)?
    for (lbl, fmt) in [
        ("PCM16 stereo 48k", pcm_format(48_000, 2, 16)),
        ("PCM16 mono 48k", pcm_format(48_000, 1, 16)),
        ("PCM16 stereo 44.1k", pcm_format(44_100, 2, 16)),
    ] {
        let Ok(c) = dev.Activate::<IAudioClient>(CLSCTX_ALL, None) else {
            continue;
        };
        match c.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, 0, min_period, min_period, &fmt, None) {
            Ok(()) => println!("    [{label}] EXCLUSIVE {lbl}: Initialize OK  <== HARDWARE CAN STREAM"),
            Err(e) if e.code().0 == NOT_ALIGNED => println!(
                "    [{label}] EXCLUSIVE {lbl}: format accepted (needs buffer alignment) <== HARDWARE CAN STREAM"
            ),
            Err(e) => println!(
                "    [{label}] EXCLUSIVE {lbl}: FAILED {e} [0x{:08X}]",
                e.code().0 as u32
            ),
        }
    }
}

/// Activate a fresh client and attempt Initialize with the given flags/format.
/// A null `fmt` means "use the device mix format".
#[cfg(windows)]
unsafe fn try_init(
    dev: &windows::Win32::Media::Audio::IMMDevice,
    flags: u32,
    fmt: *const windows::Win32::Media::Audio::WAVEFORMATEX,
    buf_dur: i64,
    label: &str,
) {
    use windows::Win32::Media::Audio::{AUDCLNT_SHAREMODE_SHARED, IAudioClient};
    use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

    let client: IAudioClient = match dev.Activate(CLSCTX_ALL, None) {
        Ok(c) => c,
        Err(e) => {
            println!("    [{label}] Activate FAILED: {e}");
            return;
        }
    };

    // Resolve the format pointer (own the mix format if fmt is null).
    let mut mixptr: *mut windows::Win32::Media::Audio::WAVEFORMATEX = std::ptr::null_mut();
    let use_fmt = if fmt.is_null() {
        match client.GetMixFormat() {
            Ok(p) => {
                mixptr = p;
                p as *const _
            }
            Err(e) => {
                println!("    [{label}] GetMixFormat FAILED: {e}");
                return;
            }
        }
    } else {
        fmt
    };

    match client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, buf_dur, 0, use_fmt, None) {
        Ok(()) => println!("    [{label}] Initialize OK  <== WORKS"),
        Err(e) => println!("    [{label}] FAILED: {e} [0x{:08X}]", e.code().0 as u32),
    }
    if !mixptr.is_null() {
        CoTaskMemFree(Some(mixptr as *const _));
    }
}

#[cfg(windows)]
unsafe fn probe_wasapi_flow(
    en: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    flow: windows::Win32::Media::Audio::EDataFlow,
    label: &str,
    loopback: bool,
) {
    use windows::Win32::Media::Audio::{IMMDeviceCollection, DEVICE_STATE_ACTIVE};

    let coll: IMMDeviceCollection = match en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) {
        Ok(c) => c,
        Err(e) => {
            println!("\n  {label}: EnumAudioEndpoints FAILED: {e}");
            return;
        }
    };
    let n = coll.GetCount().unwrap_or(0);
    println!("\n  {label}: {n} active endpoint(s)");
    for i in 0..n {
        if let Ok(dev) = coll.Item(i) {
            print!("    [{i}] ");
            probe_wasapi_device(&dev, loopback);
        }
    }
}

#[cfg(windows)]
unsafe fn probe_wasapi_device(dev: &windows::Win32::Media::Audio::IMMDevice, loopback: bool) {
    use std::ptr;
    use windows::Win32::Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioClient, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

    let id = dev
        .GetId()
        .ok()
        .map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        })
        .unwrap_or_default();
    println!("id={id}");

    let client: IAudioClient = match dev.Activate(CLSCTX_ALL, None) {
        Ok(c) => c,
        Err(e) => {
            println!("        Activate FAILED: {e}");
            return;
        }
    };
    let pformat = match client.GetMixFormat() {
        Ok(p) => p,
        Err(e) => {
            println!("        GetMixFormat FAILED: {e}");
            return;
        }
    };
    let wfx: WAVEFORMATEX = ptr::read_unaligned(pformat);
    // Copy packed fields to locals before formatting (can't reference packed fields).
    let (tag, ch, rate, bits, block_align) = (
        wfx.wFormatTag,
        wfx.nChannels,
        wfx.nSamplesPerSec,
        wfx.wBitsPerSample,
        wfx.nBlockAlign,
    );
    println!("        mix format: tag={tag} ch={ch} rate={rate} bits={bits} blockAlign={block_align}");

    let flags: u32 = if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
    match client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, pformat, None) {
        Ok(()) => println!(
            "        Initialize({}) OK",
            if loopback { "LOOPBACK" } else { "capture" }
        ),
        Err(e) => println!(
            "        Initialize({}) FAILED: {e} [0x{:08X}]",
            if loopback { "LOOPBACK" } else { "capture" },
            e.code().0 as u32
        ),
    }
    CoTaskMemFree(Some(pformat as *const _));
}

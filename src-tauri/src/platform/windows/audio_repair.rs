//! Detect and repair an impaired Windows shared-mode audio engine.
//!
//! Some machines have a broken "audio enhancement" (an Audio Processing Object)
//! wired into the *shared-mode* audio path. When it misbehaves, every shared-mode
//! `IAudioClient::Initialize` fails with `E_INVALIDARG (0x80070057)` — for the mic
//! *and* the speakers — so no recorder (ours, Zoom, even Voice Recorder) can
//! capture, while exclusive mode still works. This module:
//!
//!   * [`probe`] — reports whether shared mode works, whether exclusive works, and
//!     whether a repair is warranted, without changing anything.
//!   * [`repair`] — disables the offending enhancement on the default capture and
//!     render endpoints and restarts Windows Audio, via an elevated PowerShell
//!     (the user approves the UAC prompt). Reversible; installs nothing.
//!   * [`open_sound_settings`] — opens the classic Sound control panel as a manual
//!     fallback.
//!
//! COM work runs on a dedicated thread so we control its apartment (MTA) instead
//! of inheriting whatever the caller thread was initialized as.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use windows::core::PWSTR;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};

use crate::models::AudioHealth;

/// The MMDevices property that disables system effects (audio enhancements) for an
/// endpoint. Setting it to 1 bypasses the shared-mode APO chain.
const DISABLE_SYSFX: &str = "{e0f158e1-cb04-4c4b-aefd-f2df462b0c1c},5";

/// AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED — the format *is* accepted, only the buffer
/// size differs, so we treat it as "this format works" for probing purposes.
const NOT_ALIGNED: windows::core::HRESULT = windows::core::HRESULT(0x8889_0019u32 as i32);

/// Probe the shared-mode audio engine. Never mutates anything.
pub fn probe() -> AudioHealth {
    std::thread::spawn(|| {
        unsafe { probe_inner() }.unwrap_or_else(|e| AudioHealth {
            supported: true,
            detail: format!("Audio probe failed: {e}"),
            ..Default::default()
        })
    })
    .join()
    .unwrap_or_else(|_| AudioHealth {
        supported: true,
        detail: "Audio probe crashed.".into(),
        ..Default::default()
    })
}

unsafe fn probe_inner() -> Result<AudioHealth, Box<dyn std::error::Error>> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

    let mic = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok();
    let render = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok();

    let mic_shared = mic.as_ref().map(|d| shared_init_ok(d, false)).unwrap_or(false);
    let render_shared = render
        .as_ref()
        .map(|d| shared_init_ok(d, true))
        .unwrap_or(false);
    // Healthy = the mic (and, if present, the render/loopback endpoint) init in
    // shared mode. That is the normal path both we and conferencing apps use.
    let shared_ok = mic_shared && render.as_ref().map(|_| render_shared).unwrap_or(true);

    let exclusive_ok = mic.as_ref().map(|d| exclusive_init_ok(d)).unwrap_or(false);

    let needs_repair = !shared_ok;
    let detail = if shared_ok {
        "Shared-mode audio is healthy — the microphone and system audio record normally."
            .to_string()
    } else if exclusive_ok {
        "Windows shared-mode audio is impaired (a broken audio enhancement). The mic still \
         records via exclusive mode; repairing restores normal shared mode so it also works \
         alongside meeting apps."
            .to_string()
    } else {
        "Windows shared-mode audio failed to initialize. Try the repair, then re-check; if it \
         persists, verify the microphone and Windows privacy settings."
            .to_string()
    };

    Ok(AudioHealth {
        supported: true,
        shared_ok,
        exclusive_ok,
        needs_repair,
        detail,
    })
}

/// Does shared-mode `Initialize` succeed on this endpoint (with the loopback flag
/// for render endpoints)? This is the exact call the broken APO rejects.
unsafe fn shared_init_ok(device: &IMMDevice, loopback: bool) -> bool {
    let client: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pformat = match client.GetMixFormat() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let flags = if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
    let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, pformat, None);
    CoTaskMemFree(Some(pformat as *const c_void));
    init.is_ok()
}

/// Can the mic be opened in exclusive mode with any common PCM format? Exclusive
/// mode bypasses the shared engine, so success here means capture is still possible
/// even when shared mode is broken.
unsafe fn exclusive_init_ok(device: &IMMDevice) -> bool {
    let (native_rate, native_ch) = {
        let probe: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
            Ok(c) => c,
            Err(_) => return false,
        };
        match probe.GetMixFormat() {
            Ok(p) => {
                let w = ptr::read_unaligned(p);
                let out = (w.nSamplesPerSec, w.nChannels.max(1));
                CoTaskMemFree(Some(p as *const c_void));
                out
            }
            Err(_) => (48_000, 2),
        }
    };
    let min_period = {
        let probe: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut default_period = 0i64;
        let mut min_period = 0i64;
        if probe
            .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))
            .is_err()
        {
            return false;
        }
        if min_period <= 0 { 30_000 } else { min_period }
    };

    let candidates = [
        (native_rate, native_ch, 16u16),
        (48_000, 2, 16),
        (44_100, 2, 16),
        (native_rate, 1, 16),
        (48_000, 1, 16),
    ];
    for &(rate, ch, bits) in &candidates {
        let format = pcm_format(rate, ch, bits);
        let client: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
            Ok(c) => c,
            Err(_) => continue,
        };
        match client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            min_period,
            min_period,
            &format,
            None,
        ) {
            Ok(_) => return true,
            // Format accepted; only the buffer size needs re-aligning → it works.
            Err(e) if e.code() == NOT_ALIGNED => return true,
            Err(_) => continue,
        }
    }
    false
}

/// Disable the broken enhancement on the default endpoints and restart Windows
/// Audio, elevated via UAC. Returns a status message on successful *launch* (the
/// elevated work runs asynchronously; the UI should re-probe afterwards).
pub fn repair() -> Result<String, String> {
    std::thread::spawn(|| unsafe { repair_inner() })
        .join()
        .unwrap_or_else(|_| Err("Audio repair crashed.".into()))
}

unsafe fn repair_inner() -> Result<String, String> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;

    let cap_id = enumerator
        .GetDefaultAudioEndpoint(eCapture, eConsole)
        .ok()
        .and_then(|d| endpoint_id(&d));
    let ren_id = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .ok()
        .and_then(|d| endpoint_id(&d));

    if cap_id.is_none() && ren_id.is_none() {
        return Err("No default audio endpoints were found to repair.".into());
    }

    let script = build_repair_script(cap_id.as_deref(), ren_id.as_deref());
    let path = std::env::temp_dir().join("meetapp-audio-repair.ps1");
    std::fs::write(&path, script).map_err(|e| format!("Couldn't write repair script: {e}"))?;
    launch_elevated(&path)?;
    Ok("Audio repair launched. Approve the Windows prompt, then use “Re-check”.".into())
}

/// The registry key name for an endpoint matches its device id verbatim.
unsafe fn endpoint_id(device: &IMMDevice) -> Option<String> {
    let p = device.GetId().ok()?;
    let s = pwstr_to_string(p);
    CoTaskMemFree(Some(p.0 as *const c_void));
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

unsafe fn pwstr_to_string(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.0.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p.0, len))
}

fn build_repair_script(cap: Option<&str>, ren: Option<&str>) -> String {
    let mut s = String::new();
    s.push_str("$ErrorActionPreference = 'Continue'\n");
    s.push_str(&format!("$disable = '{DISABLE_SYSFX}'\n"));
    if let Some(id) = cap {
        push_endpoint(&mut s, "Capture", id);
    }
    if let Some(id) = ren {
        push_endpoint(&mut s, "Render", id);
    }
    // -Force restarts dependent services (AudioEndpointBuilder) too.
    s.push_str("Restart-Service -Name Audiosrv -Force -ErrorAction SilentlyContinue\n");
    s
}

fn push_endpoint(s: &mut String, flow: &str, id: &str) {
    // Endpoint ids are registry-safe (braces/dots only); escape single quotes anyway.
    let id = id.replace('\'', "''");
    s.push_str(&format!(
        "$p = 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\{flow}\\{id}\\FxProperties'\n"
    ));
    s.push_str("if (-not (Test-Path $p)) { New-Item -Path $p -Force | Out-Null }\n");
    s.push_str("New-ItemProperty -Path $p -Name $disable -PropertyType DWord -Value 1 -Force | Out-Null\n");
}

fn launch_elevated(script_path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let path = script_path.to_string_lossy().replace('\'', "''");
    let inner = format!(
        "Start-Process powershell -Verb RunAs -WindowStyle Hidden -ArgumentList \
         @('-NoProfile','-ExecutionPolicy','Bypass','-File','{path}')"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &inner])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Couldn't launch the elevated repair: {e}"))?;
    Ok(())
}

/// Open the classic Sound control panel (Recording/Playback → Advanced → audio
/// enhancements) so the user can toggle enhancements by hand.
pub fn open_sound_settings() -> Result<(), String> {
    std::process::Command::new("control")
        .arg("mmsys.cpl")
        .spawn()
        .map_err(|e| format!("Couldn't open Sound settings: {e}"))?;
    Ok(())
}

/// Build a simple integer-PCM `WAVEFORMATEX`.
fn pcm_format(rate: u32, channels: u16, bits: u16) -> WAVEFORMATEX {
    let block_align = channels * (bits / 8);
    WAVEFORMATEX {
        wFormatTag: 0x0001, // WAVE_FORMAT_PCM
        nChannels: channels,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * block_align as u32,
        nBlockAlign: block_align,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

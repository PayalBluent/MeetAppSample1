//! Standalone audio recorder: captures the microphone and the speaker
//! (system/loopback) audio via `cpal` and writes each to its own `.wav` file.
//!
//! Design rules (see the implementation spec):
//!  1. The `cpal::Stream` is owned by a long-lived background thread — never
//!     dropped early, so capture cannot die silently.
//!  2. The audio callback only pushes samples into the `hound` writer; lifetime
//!     is owned by the thread.
//!  3. The format is read from the device (`default_*_config`) — never hardcoded.
//!  4. All OS differences are isolated in `select_loopback_device` /
//!     `loopback_config`; everything else is shared.
//!  5. A peak meter in the callback distinguishes "capture dead" from
//!     "writer broken".

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SupportedStreamConfig};

/// Which source to record.
#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    Microphone,
    Speaker, // system / loopback audio
}

/// One active recording session.
struct Session {
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

/// The state Tauri owns. Holds any active sessions so their threads
/// (and therefore their streams) stay alive.
#[derive(Default)]
pub struct RecorderState {
    sessions: Mutex<Vec<Session>>,
}

// ── OS-specific device selection — the ONLY place platforms differ ──────────

/// Pick the device and its real config for a given source on the current OS.
fn select_device(source: Source) -> Result<(Device, SupportedStreamConfig), String> {
    let host = cpal::default_host();

    let device = match source {
        Source::Microphone => host
            .default_input_device()
            .ok_or("No default input (microphone) device found")?,
        Source::Speaker => select_loopback_device(&host)?,
    };

    // Read the REAL config from the device — never hardcode.
    let config = match source {
        Source::Microphone => device
            .default_input_config()
            .map_err(|e| format!("No default input config: {e}"))?,
        Source::Speaker => loopback_config(&device)?,
    };

    Ok((device, config))
}

#[cfg(target_os = "windows")]
fn select_loopback_device(host: &cpal::Host) -> Result<Device, String> {
    // On WASAPI the default OUTPUT device is captured in loopback mode by
    // building an INPUT stream on it.
    host.default_output_device()
        .ok_or_else(|| "No default output device for loopback".to_string())
}

#[cfg(target_os = "windows")]
fn loopback_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    device
        .default_output_config()
        .map_err(|e| format!("No loopback config: {e}"))
}

#[cfg(target_os = "macos")]
fn select_loopback_device(host: &cpal::Host) -> Result<Device, String> {
    // macOS 14.6+: cpal exposes system-audio loopback via CoreAudio. On older
    // macOS, install a virtual device (e.g. BlackHole), set it as the system
    // output, and select it here by name from `host.input_devices()`.
    host.default_output_device()
        .ok_or_else(|| "No loopback device. On macOS <14.6 install BlackHole.".to_string())
}

#[cfg(target_os = "macos")]
fn loopback_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    device
        .default_output_config()
        .map_err(|e| format!("No loopback config: {e}"))
}

#[cfg(target_os = "linux")]
fn select_loopback_device(host: &cpal::Host) -> Result<Device, String> {
    // With the pulseaudio/pipewire feature, the monitor source appears as an
    // INPUT device whose name contains "monitor". Pick that — never `default`.
    let mut fallback = None;
    for device in host.input_devices().map_err(|e| e.to_string())? {
        if let Ok(name) = device.name() {
            if name.to_lowercase().contains("monitor") {
                return Ok(device);
            }
            fallback = Some(device);
        }
    }
    fallback
        .ok_or_else(|| "No monitor source found. Ensure PulseAudio/PipeWire is running.".to_string())
}

#[cfg(target_os = "linux")]
fn loopback_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    // A monitor source is an input device, so its input config applies.
    device
        .default_input_config()
        .map_err(|e| format!("No loopback config: {e}"))
}

// ── Shared capture logic (identical on every OS) ────────────────────────────

/// WAV spec derived from the device: float sources are written as 32-bit float,
/// integer sources as 16-bit PCM. Channels and sample rate come from the device.
fn wav_spec(config: &SupportedStreamConfig) -> hound::WavSpec {
    let is_float = config.sample_format().is_float();
    hound::WavSpec {
        channels: config.channels(),
        sample_rate: config.sample_rate().0,
        bits_per_sample: if is_float { 32 } else { 16 },
        sample_format: if is_float {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    }
}

type Writer = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

fn err_fn(e: cpal::StreamError) {
    eprintln!("[recorder] stream error: {e}");
}

fn build_stream(
    device: &Device,
    config: &SupportedStreamConfig,
    writer: Writer,
    peak: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let cfg = config.config();

    // Speaker on Windows/macOS = build an INPUT stream on the OUTPUT device;
    // cpal enables loopback internally when `build_input_stream` is called on an
    // output device via the WASAPI/CoreAudio host.
    //
    // Every source is written as f32 (float devices) or 16-bit PCM (int devices)
    // — the two formats `hound` supports directly. `$peak` maps a sample to its
    // amplitude in [-1, 1] for the diagnostic meter; `$out` maps it to the sample
    // actually written.
    macro_rules! stream {
        ($t:ty, $peak:expr, $out:expr) => {{
            let w = writer.clone();
            let p = peak.clone();
            let to_peak = $peak;
            let to_out = $out;
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[$t], _| {
                        let mut local_peak = 0f32;
                        if let Ok(mut guard) = w.lock() {
                            if let Some(wr) = guard.as_mut() {
                                for &s in data {
                                    let a = to_peak(s).abs();
                                    if a > local_peak {
                                        local_peak = a;
                                    }
                                    let _ = wr.write_sample(to_out(s));
                                }
                            }
                        }
                        p.store(local_peak.to_bits(), Ordering::Relaxed);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build_input_stream failed: {e}"))
        }};
    }

    match config.sample_format() {
        SampleFormat::F32 => stream!(f32, |s: f32| s, |s: f32| s),
        SampleFormat::F64 => stream!(f64, |s: f64| s as f32, |s: f64| s as f32),
        SampleFormat::I16 => stream!(i16, |s: i16| s as f32 / 32_768.0, |s: i16| s),
        SampleFormat::U16 => stream!(
            u16,
            |s: u16| (s as f32 / 32_768.0) - 1.0,
            |s: u16| (s as i32 - 32_768) as i16
        ),
        SampleFormat::I32 => stream!(i32, |s: i32| s as f32 / 2_147_483_648.0, |s: i32| (s >> 16) as i16),
        SampleFormat::I8 => stream!(i8, |s: i8| s as f32 / 128.0, |s: i8| (s as i16) << 8),
        SampleFormat::U8 => stream!(
            u8,
            |s: u8| (s as f32 / 128.0) - 1.0,
            |s: u8| ((s as i16 - 128) << 8)
        ),
        other => Err(format!("Unsupported sample format: {other:?}")),
    }
}

/// Start recording `source` into `out_path`. Returns once capture has actually
/// started (or reports the setup error); capture runs on a background thread
/// until the returned `Session` is stopped.
fn start_session(source: Source, out_path: PathBuf) -> Result<Session, String> {
    let (device, config) = select_device(source)?;
    println!(
        "[recorder] {} -> {:?} | {:?}",
        match source {
            Source::Microphone => "mic",
            Source::Speaker => "speaker",
        },
        out_path,
        config
    );

    let spec = wav_spec(&config);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU32::new(0));

    let (ready_tx, ready_rx): (Sender<Result<(), String>>, _) = mpsc::channel();
    let thread_stop = stop_flag.clone();
    let thread_peak = peak.clone();

    let handle = std::thread::spawn(move || {
        // Create the writer INSIDE the thread so it lives with the stream.
        let writer: Writer = match hound::WavWriter::create(&out_path, spec) {
            Ok(w) => Arc::new(Mutex::new(Some(w))),
            Err(e) => {
                let _ = ready_tx.send(Err(format!("WavWriter::create failed: {e}")));
                return;
            }
        };

        let stream = match build_stream(&device, &config, writer.clone(), thread_peak) {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };

        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(format!("stream.play failed: {e}")));
            return;
        }

        // Setup succeeded — the (!Send) stream now lives on this thread.
        let _ = ready_tx.send(Ok(()));

        while !thread_stop.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Stop capture and finalize the file (writes the WAV header correctly).
        drop(stream);
        let finished = writer.lock().ok().and_then(|mut g| g.take());
        if let Some(w) = finished {
            if let Err(e) = w.finalize() {
                eprintln!("[recorder] finalize failed: {e}");
            }
        }
    });

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Session {
            stop_flag,
            handle: Some(handle),
        }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err("Capture thread died during setup".to_string()),
    }
}

impl Session {
    fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ── Tauri commands (called from the frontend) ───────────────────────────────

#[tauri::command]
pub fn start_recording(
    state: tauri::State<RecorderState>,
    mic_path: String,
    speaker_path: String,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    if !sessions.is_empty() {
        return Err("Already recording".into());
    }

    let mic = start_session(Source::Microphone, PathBuf::from(mic_path))?;
    let speaker = match start_session(Source::Speaker, PathBuf::from(speaker_path)) {
        Ok(s) => s,
        Err(e) => {
            mic.stop(); // don't leave a half-started recording running
            return Err(format!("Speaker capture failed: {e}"));
        }
    };

    sessions.push(mic);
    sessions.push(speaker);
    Ok(())
}

#[tauri::command]
pub fn stop_recording(state: tauri::State<RecorderState>) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    for session in sessions.drain(..) {
        session.stop();
    }
    Ok(())
}

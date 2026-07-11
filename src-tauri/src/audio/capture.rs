//! Audio capture sources. Each source runs on its own thread, timestamps every
//! buffer the instant it arrives, and pushes an [`AudioPacket`] onto the shared
//! queue. **No processing happens here** — that's the pipeline's job.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{AudioPacket, AudioSource};

/// Start microphone capture. On Windows the primary path is a raw WASAPI capture
/// endpoint (shared mode, then exclusive mode as a fallback for machines whose
/// shared audio engine is impaired); cpal is the last-resort fallback. On other
/// platforms cpal is used. Returns the capture thread plus `exclusive = true` when
/// the mic had to open in exclusive mode, so callers can warn that other apps
/// (Zoom/Teams) may lose the microphone while recording.
pub fn spawn_microphone(
    tx: Sender<AudioPacket>,
    stop: Arc<AtomicBool>,
) -> Option<(JoinHandle<()>, bool)> {
    #[cfg(windows)]
    {
        if let Some((h, exclusive)) =
            crate::platform::windows::system_audio::start_microphone(tx.clone(), stop.clone())
        {
            return Some((h, exclusive));
        }
        tracing::warn!("microphone: wasapi capture unavailable, falling back to cpal");
        cpal_microphone(tx, stop).map(|h| (h, false))
    }
    #[cfg(not(windows))]
    {
        cpal_microphone(tx, stop).map(|h| (h, false))
    }
}

/// cpal microphone capture. Enumerates **every** input device (not just the
/// default), tries each device's default and supported formats, and — crucially —
/// only commits to a stream once its callback has actually delivered PCM samples.
/// Every stage is logged, so a failure says exactly which device/format/step
/// failed and why. Returns the live capture thread, or `None` if nothing worked.
#[cfg(feature = "mic-capture")]
fn cpal_microphone(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::collections::HashSet;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    // On success we report the chosen device name; on failure, the last error.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
    let join = std::thread::Builder::new()
        .name("meetapp-cpal-mic".into())
        .spawn(move || {
            let host = cpal::default_host();
            tracing::info!("mic(cpal): host = {:?}", host.id());

            // Candidate devices: default first, then every other input device.
            let mut devices: Vec<cpal::Device> = Vec::new();
            match host.default_input_device() {
                Some(d) => {
                    tracing::info!("mic(cpal): default input device = {:?}", d.name().ok());
                    devices.push(d);
                }
                None => tracing::warn!("mic(cpal): no default input device"),
            }
            match host.input_devices() {
                Ok(list) => devices.extend(list),
                Err(e) => tracing::warn!("mic(cpal): input_devices() error: {e}"),
            }
            if devices.is_empty() {
                let _ = ready_tx.send(Err("no input devices enumerated".into()));
                return;
            }
            tracing::info!("mic(cpal): {} candidate device(s)", devices.len());

            let mut seen = HashSet::new();
            let mut last_err = "no input device produced PCM samples".to_string();
            for device in &devices {
                let name = device.name().unwrap_or_else(|_| "<unknown>".into());
                if !seen.insert(name.clone()) {
                    continue; // default device also appears in the full list
                }
                for (fmt, config) in candidate_configs(device) {
                    tracing::info!(
                        "mic(cpal): '{name}' trying {fmt:?} {}ch @ {}Hz",
                        config.channels, config.sample_rate.0
                    );
                    let frames = Arc::new(AtomicUsize::new(0));
                    let stream = match open_input_stream(device, fmt, &config, tx.clone(), frames.clone()) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("mic(cpal): '{name}' build_input_stream failed: {e}");
                            last_err = format!("{name}: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = stream.play() {
                        tracing::warn!("mic(cpal): '{name}' play() failed: {e}");
                        last_err = format!("{name}: play: {e}");
                        continue;
                    }

                    // VALIDATE: don't declare success until the callback has
                    // actually delivered samples. A stream can "play" yet never
                    // fire — that used to look like success and record silence.
                    let mut waited = 0u64;
                    let delivered = loop {
                        if frames.load(Ordering::Relaxed) > 0 {
                            break true;
                        }
                        if waited >= 1500 {
                            break false;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                        waited += 50;
                    };
                    if !delivered {
                        tracing::warn!(
                            "mic(cpal): '{name}' opened but callback delivered NO PCM in {waited}ms — next"
                        );
                        last_err = format!("{name}: opened but produced no PCM samples");
                        drop(stream);
                        continue;
                    }

                    tracing::info!(
                        "mic(cpal): ✔ CAPTURING from '{name}' ({fmt:?} {}ch @ {}Hz) — validated {} samples",
                        config.channels, config.sample_rate.0, frames.load(Ordering::Relaxed)
                    );
                    let _ = ready_tx.send(Ok(name.clone()));
                    // The (!Send) stream must live on this thread until stop.
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(stream);
                    return;
                }
            }
            let _ = ready_tx.send(Err(last_err));
        })
        .ok()?;

    match ready_rx.recv() {
        Ok(Ok(name)) => {
            tracing::info!("microphone capture active (cpal): {name}");
            Some(join)
        }
        Ok(Err(e)) => {
            tracing::warn!("cpal microphone unavailable: {e}");
            let _ = join.join();
            None
        }
        Err(_) => {
            let _ = join.join();
            None
        }
    }
}

/// Formats to try for a device: its default config first, then every other
/// supported config (at its max sample rate), de-duplicated.
#[cfg(feature = "mic-capture")]
fn candidate_configs(device: &cpal::Device) -> Vec<(cpal::SampleFormat, cpal::StreamConfig)> {
    use cpal::traits::DeviceTrait;
    let mut out: Vec<(cpal::SampleFormat, cpal::StreamConfig)> = Vec::new();
    if let Ok(def) = device.default_input_config() {
        out.push((def.sample_format(), def.into()));
    }
    if let Ok(ranges) = device.supported_input_configs() {
        for r in ranges {
            let sc = r.with_max_sample_rate();
            let fmt = sc.sample_format();
            let cfg: cpal::StreamConfig = sc.into();
            if !out
                .iter()
                .any(|(f, c)| *f == fmt && c.channels == cfg.channels && c.sample_rate == cfg.sample_rate)
            {
                out.push((fmt, cfg));
            }
        }
    }
    out
}

#[cfg(feature = "mic-capture")]
fn mic_err(e: cpal::StreamError) {
    tracing::error!("mic(cpal): stream error: {e}");
}

/// Build a cpal input stream for the given format, converting every sample type
/// to f32 and counting delivered samples in `frames` (used to validate the
/// callback actually fires before we commit to this device).
#[cfg(feature = "mic-capture")]
fn open_input_stream(
    device: &cpal::Device,
    fmt: cpal::SampleFormat,
    config: &cpal::StreamConfig,
    tx: Sender<AudioPacket>,
    frames: Arc<std::sync::atomic::AtomicUsize>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let sample_rate = config.sample_rate.0;
    let channels = config.channels;
    let result = match fmt {
        cpal::SampleFormat::F32 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples: data.to_vec(),
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| s as f32 / 32_768.0).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[u16], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| (s as f32 / 32_768.0) - 1.0).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::I8 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[i8], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| s as f32 / 128.0).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::U8 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[u8], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| (s as f32 / 128.0) - 1.0).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::I32 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[i32], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        cpal::SampleFormat::F64 => {
            let f = frames.clone();
            device.build_input_stream(
                config,
                move |data: &[f64], _| {
                    f.fetch_add(data.len(), Ordering::Relaxed);
                    let samples = data.iter().map(|&s| s as f32).collect();
                    let _ = tx.send(AudioPacket {
                        timestamp: Instant::now(),
                        sample_rate,
                        channels,
                        samples,
                        source: AudioSource::Microphone,
                    });
                },
                mic_err,
                None,
            )
        }
        other => return Err(format!("unsupported sample format {other:?}")),
    };
    result.map_err(|e| e.to_string())
}

#[cfg(not(feature = "mic-capture"))]
fn cpal_microphone(_tx: Sender<AudioPacket>, _stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    None
}

/// Start system-audio capture, behind a common interface. Only Windows (WASAPI)
/// is implemented; other platforms return `None` for now.
pub fn spawn_system_audio(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    #[cfg(windows)]
    {
        crate::platform::windows::system_audio::start(tx, stop)
    }
    #[cfg(not(windows))]
    {
        let _ = (tx, stop);
        None
    }
}

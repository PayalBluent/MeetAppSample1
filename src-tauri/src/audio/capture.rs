//! Audio capture sources. Each source runs on its own thread, timestamps every
//! buffer the instant it arrives, and pushes an [`AudioPacket`] onto the shared
//! queue. **No processing happens here** — that's the pipeline's job.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{AudioPacket, AudioSource};

/// Start microphone capture. Tries cpal first; if the default input device can't
/// be opened (feature disabled, driver quirk, exclusive-mode conflict), falls
/// back to a raw WASAPI capture endpoint on Windows before giving up.
pub fn spawn_microphone(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    if let Some(h) = cpal_microphone(tx.clone(), stop.clone()) {
        return Some(h);
    }
    #[cfg(windows)]
    {
        if let Some(h) = crate::platform::windows::system_audio::start_microphone(tx, stop) {
            tracing::info!("microphone: cpal unavailable, using WASAPI capture-endpoint fallback");
            return Some(h);
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = (tx, stop);
        None
    }
}

/// cpal microphone capture. Returns the capture thread's handle, or `None` if no
/// input device is available / the feature is disabled.
#[cfg(feature = "mic-capture")]
fn cpal_microphone(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::mpsc;

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let join = std::thread::Builder::new()
        .name("meetapp-cpal-mic".into())
        .spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    let _ = ready_tx.send(Err("no default input device".into()));
                    return;
                }
            };
            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            let sample_format = config.sample_format();
            let config: cpal::StreamConfig = config.into();
            let sample_rate = config.sample_rate.0;
            let channels = config.channels;

            // Each callback: timestamp immediately, normalize to f32, enqueue.
            let send = move |samples: Vec<f32>, tx: &Sender<AudioPacket>| {
                let _ = tx.send(AudioPacket {
                    timestamp: Instant::now(),
                    sample_rate,
                    channels,
                    samples,
                    source: AudioSource::Microphone,
                });
            };

            let err_fn = |e| tracing::error!("mic stream error: {e}");
            let tx_f32 = tx.clone();
            let tx_i16 = tx.clone();
            let tx_u16 = tx;
            let send_i16 = send.clone();
            let send_u16 = send.clone();
            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &config,
                    move |data: &[f32], _| send(data.to_vec(), &tx_f32),
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        send_i16(data.iter().map(|&s| s as f32 / 32_768.0).collect(), &tx_i16)
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        send_u16(
                            data.iter().map(|&s| (s as f32 / 32_768.0) - 1.0).collect(),
                            &tx_u16,
                        )
                    },
                    err_fn,
                    None,
                ),
                other => {
                    let _ = ready_tx.send(Err(format!("unsupported sample format: {other:?}")));
                    return;
                }
            };
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }

            let _ = ready_tx.send(Ok(()));
            // The (!Send) stream lives here until we're asked to stop.
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(stream);
        })
        .ok()?;

    match ready_rx.recv() {
        Ok(Ok(())) => Some(join),
        Ok(Err(e)) => {
            tracing::warn!("microphone capture unavailable: {e}");
            let _ = join.join();
            None
        }
        Err(_) => {
            let _ = join.join();
            None
        }
    }
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

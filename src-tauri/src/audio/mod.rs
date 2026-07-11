//! Audio capture pipeline.
//!
//! All audio stays in Rust. The microphone is captured with cpal and system
//! output with WASAPI (Windows); both feed **one** shared queue as timestamped
//! [`AudioPacket`]s. A single pipeline thread synchronizes, runs voice-activity
//! detection, labels speakers ("You" vs "Remote"), and writes storage:
//!
//! ```text
//! Capture (cpal mic + WASAPI system)
//!     -> Timestamp        (in the capture callback)
//!     -> Shared queue     (mpsc)
//!     -> Synchronization  (pipeline, ordered by arrival/timestamp)
//!     -> VAD + labeling
//!     -> Storage (48 kHz stereo WAV: L = system, R = mic)
//! ```
//!
//! The [`Recorder`] wires these together; the rest of the app only starts/stops
//! it and reads the input levels.

mod capture;
pub mod denoise;
mod pipeline;
mod vad;

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use std::time::Instant;

use crate::error::{AppError, AppResult};

/// Which stream a packet came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    /// Local microphone → labeled "You".
    Microphone,
    /// System output (the far end) → labeled "Remote".
    System,
}

/// A timestamped buffer of interleaved f32 samples from one source. This is the
/// single unit that flows through the whole pipeline.
pub struct AudioPacket {
    /// Captured the instant the buffer arrived, for synchronization.
    pub timestamp: Instant,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
    pub source: AudioSource,
}

/// A span during which one speaker was active, from voice-activity detection.
#[derive(Debug, Clone)]
pub struct SpeakerSegment {
    /// "You" (microphone) or "Remote" (system).
    pub speaker: &'static str,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Owns the capture threads and the processing pipeline for one recording.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    mic_join: Option<JoinHandle<()>>,
    system_join: Option<JoinHandle<()>>,
    pipeline_join: Option<JoinHandle<()>>,
    mic_level: Arc<AtomicU32>,
    sys_level: Arc<AtomicU32>,
    segments: Arc<Mutex<Vec<SpeakerSegment>>>,
    /// True when the microphone opened in exclusive mode (shared audio is impaired
    /// on this machine). Surfaced to the UI so it can warn about the conflict with
    /// conferencing apps that exclusive mode implies.
    mic_exclusive: bool,
}

impl Recorder {
    /// Begin recording to `path`. Captures the microphone always, and system
    /// output when `capture_system` is set and a backend is available. Returns
    /// `None` only if no source could be opened or the WAV can't be created.
    pub fn start(path: &Path, capture_system: bool) -> Option<Recorder> {
        let stop = Arc::new(AtomicBool::new(false));
        let mic_level = Arc::new(AtomicU32::new(0));
        let sys_level = Arc::new(AtomicU32::new(0));
        let segments = Arc::new(Mutex::new(Vec::new()));

        let (tx, rx) = mpsc::channel::<AudioPacket>();
        let start = Instant::now();

        let (mic_join, mic_exclusive) = match capture::spawn_microphone(tx.clone(), stop.clone()) {
            Some((h, exclusive)) => (Some(h), exclusive),
            None => (None, false),
        };
        let system_join = if capture_system {
            capture::spawn_system_audio(tx.clone(), stop.clone())
        } else {
            None
        };
        // Drop our own sender so the pipeline's queue disconnects once the
        // capture threads finish — that disconnect is the pipeline's stop signal.
        drop(tx);

        if mic_join.is_none() && system_join.is_none() {
            tracing::warn!("recorder: no audio source could be opened");
            return None;
        }

        let pipeline_join = pipeline::spawn(
            rx,
            path,
            start,
            mic_level.clone(),
            sys_level.clone(),
            segments.clone(),
        );
        let pipeline_join = match pipeline_join {
            Some(h) => Some(h),
            None => {
                // WAV creation failed — tear the capture threads back down.
                stop.store(true, Ordering::Relaxed);
                if let Some(h) = mic_join {
                    let _ = h.join();
                }
                if let Some(h) = system_join {
                    let _ = h.join();
                }
                return None;
            }
        };

        tracing::info!(
            "recorder started: mic={}, system={}",
            mic_join.is_some(),
            system_join.is_some()
        );

        Some(Recorder {
            stop,
            mic_join,
            system_join,
            pipeline_join,
            mic_level,
            sys_level,
            segments,
            mic_exclusive,
        })
    }

    /// Whether the microphone opened in exclusive mode (see [`Recorder::mic_exclusive`]).
    pub fn mic_exclusive(&self) -> bool {
        self.mic_exclusive
    }

    /// Current microphone input level, 0..1.
    pub fn mic_level(&self) -> f32 {
        f32::from_bits(self.mic_level.load(Ordering::Relaxed))
    }

    /// Current system-output level, 0..1.
    pub fn system_level(&self) -> f32 {
        f32::from_bits(self.sys_level.load(Ordering::Relaxed))
    }

    /// Stop capture, finalize the WAV, and return the detected speaker segments.
    pub fn stop(mut self) -> Vec<SpeakerSegment> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.mic_join.take() {
            let _ = h.join();
        }
        if let Some(h) = self.system_join.take() {
            let _ = h.join();
        }
        // Capture senders are now dropped → the pipeline drains, finalizes, exits.
        if let Some(h) = self.pipeline_join.take() {
            let _ = h.join();
        }
        self.segments
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
}

/// Convenience: build the WAV output path for a meeting inside `dir`.
pub fn wav_path(dir: &Path, slug: &str) -> std::path::PathBuf {
    dir.join(format!("{slug}.wav"))
}

/// Offline noise-clean a recorded WAV in place: decode → downmix to mono →
/// resample to 48 kHz → RNNoise → overwrite as 48 kHz mono float WAV. No-op when
/// `enabled` is false. Never discards the recording (falls back to the raw audio
/// if suppression yields nothing).
pub fn clean_wav_file(path: &Path, enabled: bool) -> AppResult<()> {
    if !enabled {
        return Ok(());
    }
    let reader = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.into_samples::<f32>().filter_map(Result::ok).collect()
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| (s as f32 / max).clamp(-1.0, 1.0))
                .collect()
        }
    };
    if samples.is_empty() {
        return Ok(());
    }

    let mono: Vec<f32> = if channels <= 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    let mono48 = resample_to(&mono, spec.sample_rate, 48_000);
    let mut denoiser = denoise::make(true);
    let mut cleaned = denoiser.process(&mono48);
    if cleaned.is_empty() {
        cleaned = mono48;
    }

    let out_spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, out_spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for s in cleaned {
        let pcm = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(pcm)
            .map_err(|e| AppError::Audio(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// End-to-end: start the recorder, capture ~2 s, stop, and confirm a finalized
    /// 48 kHz stereo WAV was produced at roughly real time. Needs an audio backend
    /// (system loopback is enough — mic is optional).
    #[test]
    fn records_stereo_wav_at_realtime() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("recorder-test.wav");
        let _ = std::fs::remove_file(&path);

        // Needs a working audio backend. If none can open (e.g. a machine whose
        // shared-mode audio engine is broken, or a headless CI box), skip rather
        // than fail — the pipeline itself is covered by `pipeline_writes_nonzero_audio`.
        let rec = match Recorder::start(&path, true) {
            Some(r) => r,
            None => {
                eprintln!("SKIP: no audio backend available in this environment");
                return;
            }
        };
        std::thread::sleep(Duration::from_millis(2000));
        let (mic, sys) = (rec.mic_level(), rec.system_level());
        let segments = rec.stop();

        let reader = hound::WavReader::open(&path).expect("WAV should exist");
        let spec = reader.spec();
        assert_eq!(spec.channels, 2, "stereo (L=system, R=mic)");
        assert_eq!(spec.sample_rate, 48_000);
        let frames = reader.len() as usize / 2;
        println!(
            "frames={frames} (~{:.2}s) mic={mic:.3} sys={sys:.3} segments={}",
            frames as f32 / 48_000.0,
            segments.len()
        );
        assert!(frames > 48_000 && frames < 48_000 * 5, "frames={frames}");
    }
}

/// Whole-buffer linear resampler (offline use).
fn resample_to(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = ((input.len() as f64) * ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 / ratio;
            let i0 = src.floor() as usize;
            let i1 = (i0 + 1).min(input.len() - 1);
            let frac = (src - i0 as f64) as f32;
            input[i0] * (1.0 - frac) + input[i1] * frac
        })
        .collect()
}

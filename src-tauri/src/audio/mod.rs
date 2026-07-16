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

mod apm;
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
    /// Flips to `true` the instant the pipeline processes its first audio packet.
    /// Device setup (WASAPI activation, mic validation) takes a moment, so this
    /// lets the UI show a "starting…" state until capture is actually live.
    audio_ready: Arc<AtomicBool>,
    /// True when the microphone opened in exclusive mode (shared audio is impaired
    /// on this machine). Surfaced to the UI so it can warn about the conflict with
    /// conferencing apps that exclusive mode implies.
    mic_exclusive: bool,
    /// True when system capture is on the endpoint-loopback fallback (device-bound,
    /// mute-sensitive) rather than the mute-immune process-loopback path. When set,
    /// the recorder watches the output-device mute state and warns on mute.
    system_endpoint_fallback: bool,
}

impl Recorder {
    /// Begin recording to `path`. Captures the microphone always, and system
    /// output when `capture_system` is set and a backend is available. Returns
    /// `None` only if no source could be opened or the WAV can't be created.
    pub fn start(path: &Path, capture_system: bool, gain: Arc<AtomicU32>) -> Option<Recorder> {
        let stop = Arc::new(AtomicBool::new(false));
        let mic_level = Arc::new(AtomicU32::new(0));
        let sys_level = Arc::new(AtomicU32::new(0));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let audio_ready = Arc::new(AtomicBool::new(false));

        let (tx, rx) = mpsc::channel::<AudioPacket>();
        let start = Instant::now();

        let (mic_join, mic_exclusive) = match capture::spawn_microphone(tx.clone(), stop.clone()) {
            Some((h, exclusive)) => (Some(h), exclusive),
            None => (None, false),
        };
        let (system_join, system_endpoint_fallback) = if capture_system {
            match capture::spawn_system_audio(tx.clone(), stop.clone()) {
                Some((h, fallback)) => (Some(h), fallback),
                None => (None, false),
            }
        } else {
            (None, false)
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
            audio_ready.clone(),
            gain,
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
            audio_ready,
            mic_exclusive,
            system_endpoint_fallback,
        })
    }

    /// Whether the microphone opened in exclusive mode (see [`Recorder::mic_exclusive`]).
    pub fn mic_exclusive(&self) -> bool {
        self.mic_exclusive
    }

    /// Whether system capture is on the mute-sensitive endpoint-loopback fallback.
    /// When true, callers should watch the output-device mute state (a muted
    /// endpoint makes this path record silence); process loopback is immune.
    pub fn system_on_endpoint_fallback(&self) -> bool {
        self.system_endpoint_fallback
    }

    /// Whether capture is live yet — `true` once the first audio packet has been
    /// processed. `false` during the brief device-setup window after `start`.
    pub fn audio_ready(&self) -> bool {
        self.audio_ready.load(Ordering::Relaxed)
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

/// Offline noise-clean a recorded WAV in place, honoring the two independent
/// toggles. The recorder writes stereo **L = system ("others"), R = mic ("you")**,
/// so cleaning is done *per channel*: `cancel_mic` runs RNNoise on the mic side
/// only, `cancel_system` on the far-end side only. RNNoise is trained on
/// single-speaker speech, so cleaning the far end (which may be music or several
/// remote speakers) is deliberately opt-in rather than forced on the whole mix.
/// The (possibly cleaned) sides are then mixed to a 48 kHz mono file — the most
/// audible form for playback and transcription.
///
/// A no-op when both toggles are off (the later normalize pass still produces the
/// mono file). Never discards audio: if suppression yields nothing on a side, the
/// raw side is kept. A mono input (e.g. re-cleaning an already-mixed file) can't
/// be separated, so it's cleaned as a whole when either toggle is on.
pub fn clean_wav_file(path: &Path, cancel_system: bool, cancel_mic: bool) -> AppResult<()> {
    if !cancel_system && !cancel_mic {
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

    let mono48: Vec<f32> = if channels >= 2 {
        // Deinterleave the two recorded sides (channel 0 = system, 1 = mic),
        // resample each to the 48 kHz RNNoise needs, clean the requested side(s),
        // then mix down. When only one side is cleaned it is shorter by RNNoise's
        // one-frame (~10 ms) fade-in; for two distinct speakers mixed to mono that
        // skew is inaudible, so we align on the shorter length.
        let frames = samples.len() / channels;
        let mut system = Vec::with_capacity(frames);
        let mut mic = Vec::with_capacity(frames);
        for f in samples.chunks(channels) {
            system.push(f[0]);
            mic.push(f.get(1).copied().unwrap_or(0.0));
        }
        let system = maybe_denoise(resample_to(&system, spec.sample_rate, 48_000), cancel_system);
        let mic = maybe_denoise(resample_to(&mic, spec.sample_rate, 48_000), cancel_mic);
        let n = system.len().min(mic.len());
        (0..n).map(|i| (system[i] + mic[i]) * 0.5).collect()
    } else {
        maybe_denoise(resample_to(&samples, spec.sample_rate, 48_000), true)
    };
    if mono48.is_empty() {
        return Ok(());
    }

    let out_spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, out_spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for s in mono48 {
        let pcm = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(pcm)
            .map_err(|e| AppError::Audio(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}

/// Run RNNoise over 48 kHz mono when `enabled`, keeping the raw input if
/// suppression yields nothing; a passthrough when disabled.
fn maybe_denoise(mono48: Vec<f32>, enabled: bool) -> Vec<f32> {
    if !enabled {
        return mono48;
    }
    let mut denoiser = denoise::make(true);
    let cleaned = denoiser.process(&mono48);
    if cleaned.is_empty() {
        mono48
    } else {
        cleaned
    }
}

/// Enhance a recorded WAV **in place** so quiet recordings become clearly audible.
///
/// Measures the recording's RMS (average) level and applies the single gain needed
/// to bring it to a loud, clear target ([`TARGET_RMS`]). A memory-less **soft
/// limiter** rounds off only the occasional peak that would exceed full scale, so
/// the body of the speech is scaled purely linearly — louder, never distorted, and
/// (unlike an adaptive AGC) mathematically incapable of collapsing the signal.
/// Only ever boosts, never attenuates; a no-op on silence.
///
/// Down-mixes to mono (meeting audio is speech — mono is the most-audible form and
/// best for transcription).
pub fn normalize_wav_file(path: &Path) -> AppResult<()> {
    /// Target RMS ≈ −16 dBFS: a loud, clearly-audible speech level.
    const TARGET_RMS: f32 = 0.16;
    /// Cap on the boost, so near-silence isn't amplified into a wall of noise.
    const MAX_GAIN: f32 = 12.0;
    /// Soft-limiter knee — samples below this are passed through untouched.
    const KNEE: f32 = 0.95;

    let reader = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match spec.sample_format {
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
    if interleaved.is_empty() {
        return Ok(());
    }

    let mut mono: Vec<f32> = if channels <= 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    let peak = mono.iter().fold(0f32, |m, &s| m.max(s.abs()));
    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    if peak < 1e-5 || rms < 1e-6 {
        return Ok(()); // silence — nothing to enhance
    }

    // One gain to reach the target loudness (boost-only, capped).
    let gain = (TARGET_RMS / rms).clamp(1.0, MAX_GAIN);
    tracing::info!(
        "enhance {path:?}: rms={rms:.4} peak={peak:.4} -> gain x{gain:.2}"
    );
    for s in mono.iter_mut() {
        *s = soft_limit(*s * gain, KNEE);
    }

    let out_spec = hound::WavSpec {
        channels: 1,
        sample_rate: spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, out_spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for s in mono {
        writer
            .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| AppError::Audio(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}

/// Soft limiter: pass samples below `knee` straight through (linear, undistorted)
/// and smoothly compress anything above it toward ±1.0 with a `tanh` knee, so loud
/// transients never hard-clip.
fn soft_limit(v: f32, knee: f32) -> f32 {
    let a = v.abs();
    if a <= knee {
        return v;
    }
    let over = (a - knee) / (1.0 - knee);
    v.signum() * (knee + (1.0 - knee) * over.tanh())
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
        let rec = match Recorder::start(&path, true, Arc::new(AtomicU32::new(1.0f32.to_bits()))) {
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

#[cfg(test)]
mod clean_tests {
    use super::*;

    fn write_stereo(path: &Path, frames: usize) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            // L = system tone, R = mic tone (distinct) so both sides carry signal.
            let l = (i as f32 * 0.05).sin() * 0.4;
            let r = (i as f32 * 0.11).sin() * 0.4;
            w.write_sample((l * i16::MAX as f32) as i16).unwrap();
            w.write_sample((r * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// Both toggles off is a no-op: the stereo file is left untouched (the later
    /// normalize pass is what mixes to mono in that case).
    #[test]
    fn both_off_is_noop() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("clean-noop.wav");
        let _ = std::fs::remove_file(&path);
        write_stereo(&path, 48_000);

        clean_wav_file(&path, false, false).expect("no-op should succeed");

        let spec = hound::WavReader::open(&path).unwrap().spec();
        assert_eq!(spec.channels, 2, "left untouched when both toggles are off");
    }

    /// Per-channel cleaning collapses the stereo recording to a 48 kHz mono file
    /// that still contains audio (exercises deinterleave → denoise → mix → write).
    #[test]
    fn per_channel_produces_mono_audio() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("clean-mono.wav");
        let _ = std::fs::remove_file(&path);
        write_stereo(&path, 48_000);

        clean_wav_file(&path, true, true).expect("clean should succeed");

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "mixed down to mono");
        assert_eq!(spec.sample_rate, 48_000);
        let peak = reader
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 0, "cleaned mono file must still contain audio");
    }
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    /// A quiet recording is boosted to an audible level, and the boost never
    /// clips (peak stays within the ceiling) — i.e. louder, not distorted.
    #[test]
    fn quiet_recording_is_boosted_without_clipping() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("normalize.wav");
        let _ = std::fs::remove_file(&path);

        // 2 s of a very quiet 220 Hz tone (peak ~0.05) at 48 kHz mono.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..96_000 {
                let s = (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / 48_000.0).sin() * 0.05;
                w.write_sample((s * i16::MAX as f32) as i16).unwrap();
            }
            w.finalize().unwrap();
        }

        let (before_peak, before_rms) = wav_stats(&path);
        normalize_wav_file(&path).expect("normalize should succeed");
        let (after_peak, after_rms) = wav_stats(&path);

        // Much louder (AGC pulls RMS up toward the ~0.16 target)...
        assert!(
            after_rms > before_rms * 3.0 && after_rms > 0.1,
            "should be much louder: rms {before_rms:.3} -> {after_rms:.3}"
        );
        // ...but never clipping.
        assert!(after_peak <= 1.0, "must never clip, got peak {after_peak}");
    }

    fn wav_stats(path: &Path) -> (f32, f32) {
        let r = hound::WavReader::open(path).unwrap();
        let samples: Vec<f32> = r
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / 32_768.0)
            .collect();
        let peak = samples.iter().fold(0f32, |m, &s| m.max(s.abs()));
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        (peak, rms)
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

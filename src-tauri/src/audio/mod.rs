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
pub mod deepfilter;
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
    /// Set by the recorder loop while the user has the microphone muted. The
    /// pipeline reads it every buffer: the mic channel is recorded as silence and
    /// opens no speaker segment, while the system channel keeps recording.
    mic_muted: Arc<AtomicBool>,
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
        let mic_muted = Arc::new(AtomicBool::new(false));

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
            mic_muted.clone(),
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
            mic_muted,
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

    /// Set whether the user currently has the microphone muted. The pipeline reads
    /// this every buffer, so it takes effect within one buffer: the mic channel is
    /// recorded as silence (and opens no speaker segment) while set, and the system
    /// channel keeps recording. Called from the recorder loop's mute watcher.
    pub fn set_mic_muted(&self, muted: bool) {
        self.mic_muted.store(muted, Ordering::Relaxed);
    }

    /// Whether the microphone is currently muted (as last observed by the loop).
    pub fn mic_muted(&self) -> bool {
        self.mic_muted.load(Ordering::Relaxed)
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

/// Combine a video-only MP4 with a WAV audio track into a single MP4 (video copied
/// as-is, audio re-encoded to AAC) using **ffmpeg** if it's on `PATH`. Used for
/// Record Video, whose screen capture is video-only — this gives the saved video
/// its sound. Returns the combined file's path on success, or `None` when ffmpeg
/// isn't installed or the mux fails, so the caller keeps the separate files.
/// Open-source (ffmpeg); nothing is bundled.
pub fn mux_audio_into_video(video_mp4: &str, audio_wav: &str) -> Option<String> {
    use std::process::{Command, Stdio};

    let base = video_mp4.strip_suffix(".mp4").unwrap_or(video_mp4);
    let out = format!("{base}-av.mp4");

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video_mp4,
            "-i",
            audio_wav,
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-shortest",
            &out,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() && Path::new(&out).exists() => Some(out.replace('\\', "/")),
        Ok(_) => {
            tracing::warn!("ffmpeg A/V mux failed; keeping the separate video + audio files");
            None
        }
        Err(e) => {
            tracing::info!("ffmpeg not available for A/V mux ({e}); keeping separate files");
            None
        }
    }
}

/// The **single** noise-suppression decision point for the whole app.
///
/// Priority (never both, never blended):
/// ```text
///   DeepFilterNet ──ok──▶ processed audio (RNNoise not run, not initialized)
///        └──fail/unavailable──▶ RNNoise fallback ──▶ processed audio
/// ```
/// DeepFilterNet is the primary engine: it runs first and, when it succeeds, is
/// the sole source of truth. RNNoise is a safety net used *only* when DeepFilterNet
/// is unavailable (feature off / disabled), fails to run, or yields invalid output.
///
/// Operates **in place** on `path`, so the very same processed file is what gets
/// saved for playback *and* sent to transcription — there is no second, separate
/// suppression pass anywhere else in the pipeline.
///
/// The two settings toggles still gate whether we suppress at all: with both off
/// the recording is left untouched (matching the prior behavior). When either is
/// on, DeepFilterNet processes the whole file; the RNNoise fallback honors the
/// per-channel toggles.
pub fn suppress_noise(path: &Path, cancel_system: bool, cancel_mic: bool) {
    if !cancel_system && !cancel_mic {
        return; // user turned noise cancellation off entirely
    }
    match denoise_deepfilter(path) {
        Ok(()) => {
            tracing::info!("[Audio] DeepFilterNet initialized");
            tracing::info!("[Audio] Using DeepFilterNet noise suppression");
            tracing::info!("[Audio] RNNoise disabled");
        }
        Err(reason) => {
            tracing::warn!("[Audio] DeepFilterNet failed: {reason}");
            tracing::info!("[Audio] Switching to RNNoise fallback");
            if let Err(e) = clean_wav_file(path, cancel_system, cancel_mic) {
                tracing::warn!("[Audio] RNNoise fallback failed: {e}");
            }
        }
    }
}

/// Thin indirection so `suppress_noise` reads cleanly; delegates to the
/// DeepFilterNet module's in-place enhancer (subprocess, embedded model).
fn denoise_deepfilter(path: &Path) -> Result<(), String> {
    deepfilter::try_enhance_in_place(path)
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
///
/// Used as the **RNNoise fallback** inside [`suppress_noise`] and by the on-demand
/// "clean" command; it is not a second automatic suppression stage.
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
        // Gate the mic (a single speaker, so the speech-probability residual gate
        // is safe and effective); leave the far-end ungated — it may be music or
        // several remote speakers that a single-speaker gate would damage.
        let system = maybe_denoise(resample_to(&system, spec.sample_rate, 48_000), cancel_system, false);
        // High-pass the mic (~80 Hz) before suppression to strip low-frequency room
        // rumble, desk/handling thumps, and AC-hum fundamentals that RNNoise leaves
        // behind and the later loudness boost would otherwise amplify. Voice
        // fundamentals sit above this, so speech is untouched. Mic side only — the
        // far end may carry legitimate bass (music, etc.).
        let mic48 = resample_to(&mic, spec.sample_rate, 48_000);
        let mic48 = if cancel_mic { high_pass(&mic48, 48_000, 80.0) } else { mic48 };
        let mic = maybe_denoise(mic48, cancel_mic, true);
        let n = system.len().min(mic.len());
        (0..n).map(|i| (system[i] + mic[i]) * 0.5).collect()
    } else {
        // A single mixed/mono track: high-pass then suppress as the mic path.
        let mono = high_pass(&resample_to(&samples, spec.sample_rate, 48_000), 48_000, 80.0);
        maybe_denoise(mono, true, true)
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
/// suppression yields nothing; a passthrough when disabled. `gate` adds the
/// speech-probability residual gate (mic path); leave it off for the far-end,
/// whose music / multiple remote speakers a single-speaker gate would damage.
fn maybe_denoise(mono48: Vec<f32>, enabled: bool, gate: bool) -> Vec<f32> {
    if !enabled {
        return mono48;
    }
    let mut denoiser = denoise::make_gated(true, gate);
    let cleaned = denoiser.process(&mono48);
    if cleaned.is_empty() {
        mono48
    } else {
        cleaned
    }
}

/// Enhance a recorded WAV **in place** so quiet recordings become clearly audible,
/// **without amplifying the noise floor** — robust across quiet and noisy rooms.
///
/// Applies a single boost-only gain toward a loud, clear target ([`TARGET_RMS`]),
/// then a memory-less **soft limiter** rounds off only the occasional peak that
/// would exceed full scale, so speech is scaled linearly — louder, never distorted.
///
/// The gain is chosen **adaptively** from two measurements of the signal, so it
/// behaves sensibly wherever the app is used:
///   * a **noise floor** (10th-percentile short-window RMS — the quiet gaps), and
///   * a **speech level** (90th-percentile short-window RMS — the loud parts).
///
/// When those are clearly separated (real speech sitting above a noise floor), the
/// boost is additionally capped so the *post-gain noise floor stays below*
/// [`NOISE_CEILING`]. This is what stops a **quiet room / soft speaker** from being
/// pumped into a wall of hiss, while a **busy/noisy** recording (already cleaned by
/// DeepFilterNet upstream) is still lifted but never has its residual noise
/// amplified. When there's no clear separation (a near-constant signal), it falls
/// back to the plain target-loudness boost, so it never *under*-boosts genuine
/// quiet speech. Only ever boosts, never attenuates; a no-op on silence.
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
    /// Ceiling the post-gain noise floor must stay under (≈ −40 dBFS) when a real
    /// speech-above-noise separation is detected — keeps quiet rooms from pumping.
    const NOISE_CEILING: f32 = 0.01;
    /// Speech must exceed this ×noise-floor to count as "speech above a noise
    /// floor" (i.e. a bimodal signal). Below it the signal is treated as uniform
    /// and the noise-floor cap is not applied (so a soft, gap-free voice or a test
    /// tone is still boosted normally).
    const SEPARATION: f32 = 3.0;
    /// Short analysis window for the percentile measurements (20 ms @ any rate).
    const WINDOW_MS: usize = 20;

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

    // Base gain: bring overall loudness to the target (boost-only, capped). This is
    // the original behavior and the fallback when there's no clear noise floor.
    let mut gain = (TARGET_RMS / rms).clamp(1.0, MAX_GAIN);

    // Adaptive part: measure the noise floor and speech level from short-window
    // RMS percentiles. Only when speech clearly sits above a noise floor do we cap
    // the boost so the post-gain noise floor stays under NOISE_CEILING — this is
    // what keeps a quiet room / soft speaker from being pumped into audible hiss.
    let win = (spec.sample_rate as usize * WINDOW_MS / 1000).max(1);
    let mut windows: Vec<f32> = mono
        .chunks(win)
        .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
        .collect();
    let (noise_floor, speech_level) = if windows.len() >= 4 {
        windows.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pct = |p: f32| windows[((windows.len() as f32 * p) as usize).min(windows.len() - 1)];
        (pct(0.10), pct(0.90))
    } else {
        (rms, rms) // too short to profile — treat as uniform (no cap)
    };
    if noise_floor > 1e-6 && speech_level > SEPARATION * noise_floor {
        let noise_cap = (NOISE_CEILING / noise_floor).max(1.0); // never attenuate
        gain = gain.min(noise_cap);
    }

    tracing::info!(
        "enhance {path:?}: rms={rms:.4} peak={peak:.4} noise={noise_floor:.4} \
         speech={speech_level:.4} -> gain x{gain:.2}"
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

/// Trim leading and trailing dead air from a recorded WAV **in place**.
///
/// Only the silent run at the very start and very end is removed — interior gaps
/// (natural pauses, or stretches where the mic was muted while system audio kept
/// playing) are preserved, so the recording stays in sync with its transcript
/// timestamps. Silence is judged on the **combined** signal (the loudest channel at
/// each frame): a frame is dead air only when *every* channel is below the
/// threshold, so system audio during a mic-muted stretch counts as content and is
/// kept. A short pad is left on each side so speech onsets/tails aren't clipped, and
/// an all-silent file is left untouched rather than emptied.
///
/// Run this on the pre-enhancement signal (before [`normalize_wav_file`]) so
/// silence is measured before the AGC boost lifts the noise floor.
pub fn trim_silence_wav_file(path: &Path) -> AppResult<()> {
    /// Peak amplitude at/above which a frame counts as content (~ -46 dBFS).
    const THRESHOLD: f32 = 0.005;
    /// Lead-in / lead-out kept around detected content so nothing is clipped.
    const PAD_MS: u64 = 200;

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
    let total_frames = samples.len() / channels;
    if total_frames == 0 {
        return Ok(());
    }

    // Per-frame amplitude = the loudest channel at that frame, so content on *any*
    // channel (e.g. system audio while the mic is muted) keeps the frame.
    let amp = |frame: usize| -> f32 {
        let base = frame * channels;
        samples[base..base + channels]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()))
    };

    let first = (0..total_frames).find(|&f| amp(f) >= THRESHOLD);
    let last = (0..total_frames).rev().find(|&f| amp(f) >= THRESHOLD);
    let (Some(first), Some(last)) = (first, last) else {
        // Entirely silent — leave the file as-is rather than producing an empty one.
        return Ok(());
    };

    let pad = (PAD_MS * spec.sample_rate as u64 / 1000) as usize;
    let start = first.saturating_sub(pad);
    let end = (last + pad).min(total_frames - 1); // inclusive
    if start == 0 && end == total_frames - 1 {
        return Ok(()); // no dead air at either edge — nothing to trim
    }

    let out_spec = hound::WavSpec {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, out_spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for f in start..=end {
        let base = f * channels;
        for c in 0..channels {
            let pcm = (samples[base + c].clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(pcm)
                .map_err(|e| AppError::Audio(e.to_string()))?;
        }
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

    /// In the DEFAULT build (DeepFilterNet feature not compiled), the single
    /// decision point [`suppress_noise`] must fall back to RNNoise — which
    /// collapses the stereo recording to cleaned 48 kHz mono. This proves the
    /// fallback path is wired and actually runs RNNoise (not nothing). Gated to the
    /// no-feature build so DeepFilterNet can never pre-empt it here.
    #[cfg(not(feature = "deepfilter"))]
    #[test]
    fn suppress_noise_falls_back_to_rnnoise_in_default_build() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("suppress-fallback.wav");
        let _ = std::fs::remove_file(&path);
        write_stereo(&path, 48_000);

        suppress_noise(&path, true, true); // DeepFilter unavailable → RNNoise

        let spec = hound::WavReader::open(&path).unwrap().spec();
        assert_eq!(spec.channels, 1, "RNNoise fallback mixes down to mono");
        assert_eq!(spec.sample_rate, 48_000);
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

    /// Robustness: on a bimodal recording (a quiet, audible noise floor with a
    /// louder speech burst) the adaptive AGC must NOT pump the noise floor up. The
    /// old fixed-target AGC multiplied the whole file by the loudness gain, tripling
    /// the noise region; the noise-aware cap keeps the quiet region roughly where it
    /// was. This is the quiet-room / soft-speaker real-world case.
    #[test]
    fn does_not_pump_noise_floor_in_bimodal_recording() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("normalize-bimodal.wav");
        let _ = std::fs::remove_file(&path);

        let fs = 48_000usize;
        let n = fs * 2; // 2 s
        // A steady, clearly-audible noise floor (~0.03) everywhere, with a louder
        // speech-like tone burst added over the middle third.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: fs as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..n {
                let noise = (i as f32 * 0.02).sin() * 0.03;
                let speech = if i >= n / 3 && i < 2 * n / 3 {
                    (i as f32 * 0.11).sin() * 0.12
                } else {
                    0.0
                };
                w.write_sample(((noise + speech).clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .unwrap();
            }
            w.finalize().unwrap();
        }

        // Noise floor before = RMS of the quiet first third.
        let region_rms = |path: &Path, frac_lo: f32, frac_hi: f32| -> f32 {
            let s: Vec<f32> = hound::WavReader::open(path)
                .unwrap()
                .into_samples::<i16>()
                .filter_map(Result::ok)
                .map(|s| s as f32 / 32_768.0)
                .collect();
            let (lo, hi) = ((s.len() as f32 * frac_lo) as usize, (s.len() as f32 * frac_hi) as usize);
            (s[lo..hi].iter().map(|x| x * x).sum::<f32>() / (hi - lo) as f32).sqrt()
        };
        let noise_before = region_rms(&path, 0.0, 0.30);

        normalize_wav_file(&path).expect("normalize should succeed");

        let noise_after = region_rms(&path, 0.0, 0.30);
        assert!(
            noise_after <= noise_before * 1.5,
            "noise floor must not be pumped: {noise_before:.4} -> {noise_after:.4}"
        );
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

#[cfg(test)]
mod hpf_tests {
    use super::*;
    use std::f32::consts::PI;

    fn tone(freq: f32, fs: u32) -> Vec<f32> {
        (0..fs)
            .map(|i| (i as f32 * 2.0 * PI * freq / fs as f32).sin() * 0.5)
            .collect()
    }
    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    /// An 80 Hz high-pass strongly attenuates low-frequency noise (rumble/hum) but
    /// leaves the speech band essentially intact — so it cleans the mic without
    /// dulling the voice.
    #[test]
    fn high_pass_kills_rumble_keeps_speech() {
        let fs = 48_000u32;
        let low = high_pass(&tone(40.0, fs), fs, 80.0); // sub-bass rumble
        let hum = high_pass(&tone(60.0, fs), fs, 80.0); // mains hum fundamental
        let speech = high_pass(&tone(300.0, fs), fs, 80.0); // mid speech band

        // Measure the steady-state tail to skip the filter's startup transient.
        let tail = |s: &[f32]| s[fs as usize / 2..].to_vec();
        let ref40 = rms(&tail(&tone(40.0, fs)));
        let ref300 = rms(&tail(&tone(300.0, fs)));

        assert!(rms(&tail(&low)) / ref40 < 0.4, "40 Hz rumble must be cut hard");
        assert!(rms(&tail(&hum)) / ref40 < 0.7, "60 Hz hum must be attenuated");
        assert!(
            rms(&tail(&speech)) / ref300 > 0.85,
            "300 Hz speech band must be preserved"
        );
    }

    #[test]
    fn high_pass_is_noop_on_empty_or_bad_args() {
        assert!(high_pass(&[], 48_000, 80.0).is_empty());
        let s = tone(300.0, 8_000);
        assert_eq!(high_pass(&s, 0, 80.0).len(), s.len());
        assert_eq!(high_pass(&s, 48_000, 0.0).len(), s.len());
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    /// Write a mono 48 kHz WAV from `(frames, amplitude)` sections; amplitude 0.0
    /// is silence, otherwise a tone at that peak.
    fn write_mono(path: &Path, sections: &[(usize, f32)]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let mut n = 0usize;
        for &(frames, amp) in sections {
            for _ in 0..frames {
                let s = if amp > 0.0 { (n as f32 * 0.05).sin() * amp } else { 0.0 };
                w.write_sample((s * i16::MAX as f32) as i16).unwrap();
                n += 1;
            }
        }
        w.finalize().unwrap();
    }

    fn frame_count(path: &Path) -> usize {
        let r = hound::WavReader::open(path).unwrap();
        r.len() as usize / r.spec().channels.max(1) as usize
    }

    fn peak(path: &Path) -> u16 {
        hound::WavReader::open(path)
            .unwrap()
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0)
    }

    /// Leading and trailing silence is removed, the spoken content and a small pad
    /// are kept, and the file gets meaningfully shorter.
    #[test]
    fn trims_leading_and_trailing_silence() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("trim-lead-trail.wav");
        let _ = std::fs::remove_file(&path);
        // 1s silence | 1s tone | 1s silence.
        write_mono(&path, &[(48_000, 0.0), (48_000, 0.5), (48_000, 0.0)]);
        assert_eq!(frame_count(&path), 144_000);

        trim_silence_wav_file(&path).expect("trim should succeed");

        let after = frame_count(&path);
        assert!(after < 144_000, "should be trimmed shorter, got {after}");
        // ≈ 1s tone + a 200ms pad on each side.
        assert!((60_000..=68_000).contains(&after), "≈ tone + 2×0.2s pad, got {after}");
        assert!(peak(&path) > 1000, "the spoken content must be preserved");
    }

    /// An entirely silent recording is left untouched, never emptied into a broken
    /// zero-length file.
    #[test]
    fn keeps_all_silent_file_untouched() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("trim-all-silent.wav");
        let _ = std::fs::remove_file(&path);
        write_mono(&path, &[(48_000, 0.0)]);

        trim_silence_wav_file(&path).expect("trim should succeed");

        assert_eq!(frame_count(&path), 48_000, "silent file left as-is");
    }

    /// A recording with no dead air at either edge keeps its full length.
    #[test]
    fn noop_when_no_dead_air() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("trim-no-deadair.wav");
        let _ = std::fs::remove_file(&path);
        write_mono(&path, &[(48_000, 0.5)]);
        let before = frame_count(&path);

        trim_silence_wav_file(&path).expect("trim should succeed");

        assert_eq!(frame_count(&path), before, "no dead air → unchanged length");
    }

    /// Frames where only the system (left) channel has audio while the mic (right)
    /// is silent — a muted mic during active system audio — are content and must be
    /// kept: nothing is trimmed.
    #[test]
    fn keeps_frames_where_only_system_has_audio() {
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("trim-muted-mic.wav");
        let _ = std::fs::remove_file(&path);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..48_000 {
                let l = (i as f32 * 0.05).sin() * 0.5; // system audio
                let r = 0.0f32; // muted mic
                w.write_sample((l * i16::MAX as f32) as i16).unwrap();
                w.write_sample((r * i16::MAX as f32) as i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let before = frame_count(&path);

        trim_silence_wav_file(&path).expect("trim should succeed");

        assert_eq!(frame_count(&path), before, "system-only content must be kept");
    }
}

/// Second-order (biquad) high-pass filter over mono f32 samples, used to remove
/// sub-`cutoff_hz` energy from the microphone: room rumble, desk/handling thumps,
/// AC-hum fundamentals, and DC offset. Butterworth response (Q ≈ 0.707), RBJ
/// cookbook coefficients, direct-form I. Returns a buffer the same length as the
/// input; a no-op on empty input or a nonsensical rate/cutoff.
fn high_pass(input: &[f32], sample_rate: u32, cutoff_hz: f32) -> Vec<f32> {
    if input.is_empty() || sample_rate == 0 || cutoff_hz <= 0.0 {
        return input.to_vec();
    }
    let fs = sample_rate as f32;
    let q = std::f32::consts::FRAC_1_SQRT_2; // 0.707 — maximally flat (Butterworth)
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / fs;
    let (sin_w0, cos_w0) = w0.sin_cos();
    let alpha = sin_w0 / (2.0 * q);

    let a0 = 1.0 + alpha;
    let b0 = ((1.0 + cos_w0) / 2.0) / a0;
    let b1 = (-(1.0 + cos_w0)) / a0;
    let b2 = ((1.0 + cos_w0) / 2.0) / a0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;

    let mut out = Vec::with_capacity(input.len());
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for &x in input {
        let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
        out.push(y);
    }
    out
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

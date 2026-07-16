//! The single audio-processing pipeline.
//!
//! One thread owns the receiving end of the shared queue and is the *only* place
//! audio is processed. For every [`AudioPacket`] it:
//!   1. down-mixes to mono and measures level (drives the UI meters),
//!   2. runs voice-activity detection and tracks speaker segments
//!      (microphone → "You", system → "Remote"),
//!   3. resamples to 48 kHz and writes a stereo WAV (**L = system, R = mic**).
//!
//! Capture threads never process audio — they only timestamp and enqueue — so
//! this is the one spot that has to be correct.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::denoise::SpeechDetector;
use super::vad::Vad;
use super::{AudioPacket, AudioSource, SpeakerSegment};
use crate::models::MAX_INPUT_GAIN;

/// Output sample rate for the stored WAV.
const OUT_RATE: u32 = 48_000;
/// Bound on a per-channel resample queue (2 s) so a stalled source can't grow
/// memory without limit; when exceeded the excess is flushed to disk.
const MIXER_CAP: usize = OUT_RATE as usize * 2;
/// How often the storage stage flushes aligned frames.
const FLUSH: Duration = Duration::from_millis(40);

/// Spawn the processing thread. Returns `None` if the output WAV can't be created.
pub fn spawn(
    rx: Receiver<AudioPacket>,
    path: &Path,
    start: Instant,
    mic_level: Arc<AtomicU32>,
    sys_level: Arc<AtomicU32>,
    segments: Arc<Mutex<Vec<SpeakerSegment>>>,
    audio_ready: Arc<AtomicBool>,
    gain: Arc<AtomicU32>,
) -> Option<JoinHandle<()>> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: OUT_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = match hound::WavWriter::create(path, spec) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("audio pipeline: cannot create WAV {path:?}: {e}");
            return None;
        }
    };
    let path = path.to_path_buf();

    std::thread::Builder::new()
        .name("meetapp-audio-pipeline".into())
        .spawn(move || {
            run(rx, writer, path, start, mic_level, sys_level, segments, audio_ready, gain)
        })
        .ok()
}

fn run(
    rx: Receiver<AudioPacket>,
    mut writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    path: PathBuf,
    start: Instant,
    mic_level: Arc<AtomicU32>,
    sys_level: Arc<AtomicU32>,
    segments: Arc<Mutex<Vec<SpeakerSegment>>>,
    audio_ready: Arc<AtomicBool>,
    gain: Arc<AtomicU32>,
) {
    let mut mic = Channel::new("You", mic_level, gain.clone());
    let mut sys = Channel::new("Remote", sys_level, gain);
    let mut segs: Vec<SpeakerSegment> = Vec::new();
    let mut last_ms = 0u64;

    // Optional WebRTC APM (AEC/AGC/HPF) on the mic, with the system audio as the
    // echo reference. `None` when the `apm` feature is off or the processor can't
    // initialize — in which case the mic passes through untouched and the existing
    // RNNoise pass (offline) is the noise-suppression fallback.
    let mut apm = super::apm::Apm::new();
    if apm.is_some() {
        tracing::info!("audio: WebRTC APM active on the mic (AEC + AGC + high-pass)");
    }

    loop {
        match rx.recv_timeout(FLUSH) {
            Ok(pkt) => {
                last_ms = process(&pkt, start, &mut mic, &mut sys, &mut segs, &mut apm);
                // First real audio in hand — signal that capture is live so the UI
                // can drop its "starting…" state and let the user start/transcribe.
                audio_ready.store(true, Ordering::Relaxed);
                // Drain any burst without blocking so we never fall behind.
                while let Ok(pkt) = rx.try_recv() {
                    last_ms = process(&pkt, start, &mut mic, &mut sys, &mut segs, &mut apm);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        write_available(&mut writer, &mut mic, &mut sys);
    }

    // Capture stopped: flush whatever remains, then close open speaker segments.
    write_remaining(&mut writer, &mut mic, &mut sys);
    mic.seg.close(last_ms, &mut segs);
    sys.seg.close(last_ms, &mut segs);

    if let Ok(mut out) = segments.lock() {
        *out = segs;
    }
    if let Err(e) = writer.finalize() {
        tracing::error!("audio pipeline: failed to finalize WAV {path:?}: {e}");
    }

    // Verify (in the log) that each source actually delivered non-zero audio.
    let verdict = |peak: f32, packets: u64| -> &'static str {
        if packets == 0 {
            "NO PACKETS"
        } else if peak <= 0.0 {
            "SILENT (all zero)"
        } else {
            "audio OK"
        }
    };
    tracing::info!(
        "recording finalized {path:?}: mic[{} pkts, peak={:.4} -> {}], system[{} pkts, peak={:.4} -> {}]",
        mic.packets, mic.peak, verdict(mic.peak, mic.packets),
        sys.packets, sys.peak, verdict(sys.peak, sys.packets),
    );
}

/// Process one packet through level → VAD → labeling → resample stages.
/// Returns the packet's offset from `start`, in milliseconds.
fn process(
    pkt: &AudioPacket,
    start: Instant,
    mic: &mut Channel,
    sys: &mut Channel,
    segs: &mut Vec<SpeakerSegment>,
    apm: &mut Option<super::apm::Apm>,
) -> u64 {
    let now_ms = pkt.timestamp.saturating_duration_since(start).as_millis() as u64;
    let channel = match pkt.source {
        AudioSource::Microphone => mic,
        AudioSource::System => sys,
    };
    channel.push(pkt, now_ms, segs, apm);
    now_ms
}

/// Write the frames both channels have in common (the aligned overlap), keeping
/// any surplus buffered. A safety valve bounds memory if one source stalls.
fn write_available(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    mic: &mut Channel,
    sys: &mut Channel,
) {
    let frames = mic.q.len().min(sys.q.len());
    write_frames(writer, frames, mic, sys);

    // If one source stalled, the other keeps arriving; flush its excess (padding
    // the stalled channel with silence) so memory stays bounded.
    if mic.q.len() > MIXER_CAP {
        let extra = mic.q.len() - OUT_RATE as usize;
        write_frames(writer, extra, mic, sys);
    }
    if sys.q.len() > MIXER_CAP {
        let extra = sys.q.len() - OUT_RATE as usize;
        write_frames(writer, extra, mic, sys);
    }
}

fn write_remaining(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    mic: &mut Channel,
    sys: &mut Channel,
) {
    let frames = mic.q.len().max(sys.q.len());
    write_frames(writer, frames, mic, sys);
}

/// Write `frames` interleaved stereo samples: left = system, right = mic.
fn write_frames(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    frames: usize,
    mic: &mut Channel,
    sys: &mut Channel,
) {
    for _ in 0..frames {
        let r = mic.q.pop_front().unwrap_or(0.0);
        let l = sys.q.pop_front().unwrap_or(0.0);
        let _ = writer.write_sample(to_i16(l));
        let _ = writer.write_sample(to_i16(r));
    }
}

/// Convert a `[-1, 1]` float sample to 16-bit PCM.
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// Per-source processing state.
struct Channel {
    level: Arc<AtomicU32>,
    /// Capture volume, read live so the volume control takes effect mid-recording.
    gain: Arc<AtomicU32>,
    vad: Vad,
    /// Trained speech detector (RNNoise VAD probability) driving `vad`.
    detector: SpeechDetector,
    seg: SegmentTracker,
    resampler: Option<Resampler>,
    /// Resampled 48 kHz mono awaiting interleave into the WAV.
    q: VecDeque<f32>,
    /// Whether the first packet has been seen (used to align channels in time).
    primed: bool,
    /// Loudest block RMS seen — used at finalize to verify non-zero audio and
    /// how many packets actually arrived on this source.
    peak: f32,
    packets: u64,
}

impl Channel {
    fn new(speaker: &'static str, level: Arc<AtomicU32>, gain: Arc<AtomicU32>) -> Self {
        Channel {
            level,
            gain,
            vad: Vad::new(),
            detector: SpeechDetector::new(),
            seg: SegmentTracker::new(speaker),
            resampler: None,
            q: VecDeque::new(),
            primed: false,
            peak: 0.0,
            packets: 0,
        }
    }

    fn push(
        &mut self,
        pkt: &AudioPacket,
        now_ms: u64,
        segs: &mut Vec<SpeakerSegment>,
        apm: &mut Option<super::apm::Apm>,
    ) {
        // Capture volume, read fresh each buffer so the volume control is live.
        let gain = f32::from_bits(self.gain.load(Ordering::Relaxed)).clamp(0.0, MAX_INPUT_GAIN);

        let mono = downmix(&pkt.samples, pkt.channels);
        // `level` is the raw (pre-gain) loudness — used for the speech-detection
        // energy floor and the "was anything captured?" verdict, so neither is
        // skewed by the volume setting. The meter shows the *boosted* level so the
        // user sees the effect of the control.
        let level = rms(&mono);
        self.level.store((level * gain * 3.0).min(1.0).to_bits(), Ordering::Relaxed);
        self.packets += 1;
        if level > self.peak {
            self.peak = level;
        }

        // Align sources to a shared t = 0 (the recorder's start instant). A stream
        // that opens late (e.g. system loopback activation lags the mic) gets its
        // queue pre-padded with silence equal to that startup delay, so the stereo
        // channels stay in sync instead of drifting by the open-time difference.
        // Bounded by MIXER_CAP so a pathologically late source can't over-allocate.
        if !self.primed {
            self.primed = true;
            let lead = (now_ms.saturating_mul(OUT_RATE as u64) / 1000) as usize;
            self.q.extend(std::iter::repeat(0.0).take(lead.min(MIXER_CAP)));
        }

        // Resample to 48 kHz first: RNNoise's speech detector requires that rate,
        // and it's the WAV's rate anyway. Then detect *speech* on the resampled
        // mono and gate segmentation on the trained probability (not loudness),
        // so music/typing/hum never open a speaker segment.
        let rs = self
            .resampler
            .get_or_insert_with(|| Resampler::new(pkt.sample_rate, OUT_RATE));
        let mut resampled: Vec<f32> = Vec::new();
        rs.process(&mono, &mut resampled);

        // WebRTC APM stage (only when active). The far end feeds the echo
        // reference; the mic is echo-cancelled / gain-controlled / high-passed
        // before anything downstream sees it. When `apm` is `None` this is a
        // no-op and the samples are unchanged — RNNoise remains the fallback.
        if let Some(apm) = apm.as_mut() {
            match pkt.source {
                AudioSource::System => apm.push_render(&resampled),
                AudioSource::Microphone => resampled = apm.process_capture(&resampled),
            }
        }

        let speech_prob = self.detector.update(&resampled);
        let speaking = self.vad.update(speech_prob, level);
        self.seg.update(speaking, now_ms, segs);

        // Apply the capture volume to the audio we actually store. `to_i16` clamps
        // on write, so pushing gain past unity boosts quiet audio and hard-limits
        // anything that would overflow rather than wrapping.
        if (gain - 1.0).abs() < f32::EPSILON {
            self.q.extend(resampled);
        } else {
            self.q.extend(resampled.iter().map(|s| s * gain));
        }
    }
}

/// Tracks the currently-open speech segment for one speaker.
struct SegmentTracker {
    speaker: &'static str,
    open_start: Option<u64>,
}

impl SegmentTracker {
    fn new(speaker: &'static str) -> Self {
        SegmentTracker {
            speaker,
            open_start: None,
        }
    }

    fn update(&mut self, speaking: bool, now_ms: u64, out: &mut Vec<SpeakerSegment>) {
        match (self.open_start, speaking) {
            (None, true) => self.open_start = Some(now_ms),
            (Some(start), false) => {
                out.push(SpeakerSegment {
                    speaker: self.speaker,
                    start_ms: start,
                    end_ms: now_ms,
                });
                self.open_start = None;
            }
            _ => {}
        }
    }

    fn close(&mut self, now_ms: u64, out: &mut Vec<SpeakerSegment>) {
        if let Some(start) = self.open_start.take() {
            out.push(SpeakerSegment {
                speaker: self.speaker,
                start_ms: start,
                end_ms: now_ms.max(start),
            });
        }
    }
}

/// Average interleaved samples down to mono.
fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return samples.to_vec();
    }
    samples
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

fn rms(mono: &[f32]) -> f32 {
    if mono.is_empty() {
        return 0.0;
    }
    (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt()
}

/// Phase-continuous linear resampler; correct and click-free across chunk
/// boundaries and adequate for speech. This is the seam where a higher-quality
/// resampler and clock-drift correction would slot in later.
struct Resampler {
    step: f64,
    pos: f64,
    buf: Vec<f32>,
    passthrough: bool,
}

impl Resampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        let active = in_rate > 0 && out_rate > 0;
        Resampler {
            step: if active {
                in_rate as f64 / out_rate as f64
            } else {
                1.0
            },
            pos: 0.0,
            buf: Vec::new(),
            passthrough: !active || in_rate == out_rate,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend(input.iter().copied());
            return;
        }
        self.buf.extend_from_slice(input);

        let mut pos = self.pos;
        while (pos.floor() as usize) + 1 < self.buf.len() {
            let i0 = pos.floor() as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = self.buf[i0];
            let b = self.buf[i0 + 1];
            out.push(a + (b - a) * frac);
            pos += self.step;
        }
        let base = (pos.floor() as usize).min(self.buf.len());
        if base > 0 {
            self.buf.drain(0..base);
        }
        self.pos = pos - base as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo_to_mono() {
        let out = downmix(&[0.5, -0.5, 1.0, 1.0], 2);
        assert_eq!(out, vec![0.0, 1.0]);
    }

    #[test]
    fn resampler_upsamples_count() {
        let mut rs = Resampler::new(44_100, 48_000);
        let mut out: Vec<f32> = Vec::new();
        rs.process(&vec![0.1; 44_100], &mut out);
        assert!(out.len() > 47_000 && out.len() <= 48_000, "got {}", out.len());
    }

    #[test]
    fn resampler_passthrough() {
        let mut rs = Resampler::new(48_000, 48_000);
        let mut out: Vec<f32> = Vec::new();
        rs.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out.len(), 3);
    }

    /// Hardware-independent proof that the writer pipeline preserves audio: feed
    /// synthetic non-zero PCM and confirm the finalized WAV contains non-zero
    /// samples (rules out the pipeline/encoder silently zeroing the recording).
    #[test]
    fn pipeline_writes_nonzero_audio() {
        use std::sync::mpsc;
        let dir = std::env::temp_dir().join("meetapp-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("pipeline-nonzero.wav");
        let _ = std::fs::remove_file(&path);

        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        let handle = spawn(
            rx,
            &path,
            start,
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(1.0f32.to_bits())),
        )
        .expect("pipeline should start");

        // 1 s of a loud sine on the microphone source, in 480-sample packets.
        for chunk in 0..100 {
            let samples: Vec<f32> = (0..480)
                .map(|i| {
                    let n = (chunk * 480 + i) as f32;
                    (n * 0.06).sin() * 0.5
                })
                .collect();
            tx.send(AudioPacket {
                timestamp: Instant::now(),
                sample_rate: 48_000,
                channels: 1,
                samples,
                source: AudioSource::Microphone,
            })
            .unwrap();
        }
        drop(tx); // disconnect -> pipeline finalizes
        handle.join().unwrap();

        let reader = hound::WavReader::open(&path).expect("WAV should exist");
        assert_eq!(reader.spec().bits_per_sample, 16);
        let peak = reader
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 1000, "WAV must contain non-zero audio, peak={peak}");
    }

    #[test]
    fn segment_tracker_opens_and_closes() {
        let mut t = SegmentTracker::new("You");
        let mut segs = Vec::new();
        t.update(true, 100, &mut segs);
        t.update(true, 200, &mut segs);
        t.update(false, 300, &mut segs);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].speaker, "You");
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (100, 300));
    }
}

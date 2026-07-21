//! AI noise suppression.
//!
//! Default backend is RNNoise via [`nnnoiseless`] (pure-Rust, MIT — no model
//! files, no C toolchain). It's exposed behind the [`Denoiser`] trait so a
//! higher-quality DeepFilterNet backend can be dropped in later without touching
//! the capture pipeline.
//!
//! RNNoise operates on **48 kHz mono** audio in **480-sample frames**, with
//! samples scaled to the i16 range `[-32768, 32767]` (NOT `[-1, 1]`). This module
//! hides that: callers push/pull ordinary `[-1, 1]` f32 at 48 kHz and framing +
//! scaling happens internally.

/// RNNoise frame size (10 ms @ 48 kHz).
pub const FRAME_SIZE: usize = 480;

/// A streaming noise suppressor over 48 kHz mono f32 samples in `[-1, 1]`.
pub trait Denoiser: Send {
    /// Feed input samples; returns as many cleaned samples as are ready.
    /// Output may lag input by up to one frame due to internal buffering.
    fn process(&mut self, input: &[f32]) -> Vec<f32>;
    /// Label for provenance/UX.
    fn name(&self) -> &'static str {
        "passthrough"
    }
}

/// No-op denoiser (used when suppression is disabled or the feature is off).
pub struct PassThrough;

impl Denoiser for PassThrough {
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input.to_vec()
    }
}

#[cfg(feature = "denoise")]
pub use rnnoise::RnnoiseDenoiser;

/// A speech-activity detector built on RNNoise's trained voice-activity output.
///
/// RNNoise doesn't just suppress noise — every processed frame yields a
/// probability that the frame contains *speech*. Feeding it here (and reading
/// that probability, which the denoiser path discards) gives us a real
/// speech/non-speech discriminator for free: music, typing, and fan noise score
/// low, human voice scores high. This is what lets VAD detect speech instead of
/// mere loudness. Operates on 48 kHz mono, framed and scaled internally.
///
/// Without the `denoise` feature it degrades to a no-op returning `1.0`, so the
/// caller falls back to a pure energy gate.
pub struct SpeechDetector {
    #[cfg(feature = "denoise")]
    inner: rnnoise::VadState,
}

impl SpeechDetector {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "denoise")]
            inner: rnnoise::VadState::new(),
        }
    }

    /// Feed 48 kHz mono samples in `[-1, 1]`; returns the peak speech
    /// probability `[0, 1]` across the 10 ms frames this call completed, holding
    /// the previous value when the input is shorter than one frame.
    pub fn update(&mut self, samples48k: &[f32]) -> f32 {
        #[cfg(feature = "denoise")]
        {
            self.inner.update(samples48k)
        }
        #[cfg(not(feature = "denoise"))]
        {
            let _ = samples48k;
            1.0
        }
    }
}

impl Default for SpeechDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "denoise")]
mod rnnoise {
    use super::{Denoiser, FRAME_SIZE};
    use nnnoiseless::DenoiseState;

    /// Streaming wrapper that turns 48 kHz mono into RNNoise's per-frame speech
    /// probability. It runs the same net as [`RnnoiseDenoiser`] but keeps only
    /// the VAD probability and throws away the cleaned audio (that's the
    /// storage pipeline's job elsewhere).
    pub struct VadState {
        state: Box<DenoiseState<'static>>,
        /// Pending input, already scaled to the i16 range RNNoise expects.
        pending: Vec<f32>,
        /// Last reported probability, held when a call completes no full frame.
        last_prob: f32,
        /// The first frame's probability is a fade-in artifact — skip it.
        warmed: bool,
    }

    impl VadState {
        pub fn new() -> Self {
            Self {
                state: DenoiseState::new(),
                pending: Vec::with_capacity(FRAME_SIZE * 4),
                last_prob: 0.0,
                warmed: false,
            }
        }

        pub fn update(&mut self, samples48k: &[f32]) -> f32 {
            self.pending.extend(samples48k.iter().map(|s| s * 32768.0));

            let mut scratch = [0.0f32; FRAME_SIZE];
            let mut frame = [0.0f32; FRAME_SIZE];
            let mut peak = 0.0f32;
            let mut produced = false;
            while self.pending.len() >= FRAME_SIZE {
                frame.copy_from_slice(&self.pending[..FRAME_SIZE]);
                self.pending.drain(0..FRAME_SIZE);
                let prob = self.state.process_frame(&mut scratch, &frame);
                if !self.warmed {
                    self.warmed = true; // discard fade-in frame's probability
                    continue;
                }
                peak = peak.max(prob);
                produced = true;
            }
            if produced {
                self.last_prob = peak;
            }
            self.last_prob
        }
    }

    /// Speech-probability residual gate (a downward expander). RNNoise removes
    /// steady noise well but leaves audible residual *between* words; pulling
    /// noise-only frames down — with a floor so it never sounds unnaturally dead,
    /// and attack/release smoothing so it doesn't chatter — makes speech stand
    /// out. It's driven by RNNoise's trained speech probability (not raw energy),
    /// so loud non-speech (typing, clatter) is still attenuated while quiet speech
    /// is preserved.
    pub(crate) mod gate {
        /// Never attenuate below this (≈ −14 dB) — keeps a natural room floor.
        pub(crate) const FLOOR: f32 = 0.20;
        /// Speech probability below LO → noise; above HI → full speech.
        pub(crate) const LO: f32 = 0.25;
        pub(crate) const HI: f32 = 0.60;
        /// Per-frame (10 ms) smoothing coefficients: open fast, close slowly.
        pub(crate) const ATTACK: f32 = 0.7;
        pub(crate) const RELEASE: f32 = 0.10;

        /// Target gain for a speech probability (smoothstep between LO and HI).
        pub(crate) fn target(prob: f32) -> f32 {
            let t = ((prob - LO) / (HI - LO)).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t); // smoothstep
            FLOOR + (1.0 - FLOOR) * s
        }
    }

    /// RNNoise-backed suppressor. Buffers input into 480-sample frames and
    /// discards the very first output frame (RNNoise fade-in artifact). When
    /// `gate` is set, each frame is additionally scaled by the speech-probability
    /// residual gate above.
    pub struct RnnoiseDenoiser {
        state: Box<DenoiseState<'static>>,
        /// Pending input, already scaled to i16 range.
        pending: Vec<f32>,
        discarded_first: bool,
        /// Apply the speech-probability residual gate (mic path). Off for the
        /// far-end, whose music / multiple remote speakers a single-speaker gate
        /// would damage.
        gate: bool,
        /// Smoothed gate gain, carried across frames (starts open so leading
        /// speech is never clipped).
        gate_gain: f32,
    }

    impl RnnoiseDenoiser {
        /// RNNoise only (no residual gate).
        pub fn new() -> Self {
            Self::build(false)
        }

        /// RNNoise followed by the speech-probability residual gate.
        pub fn gated() -> Self {
            Self::build(true)
        }

        fn build(gate: bool) -> Self {
            Self {
                state: DenoiseState::new(),
                pending: Vec::with_capacity(FRAME_SIZE * 4),
                discarded_first: false,
                gate,
                gate_gain: 1.0,
            }
        }
    }

    impl Default for RnnoiseDenoiser {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Denoiser for RnnoiseDenoiser {
        fn process(&mut self, input: &[f32]) -> Vec<f32> {
            // Scale [-1, 1] → i16 range that RNNoise expects.
            self.pending.extend(input.iter().map(|s| s * 32768.0));

            // Walk the buffer with a read cursor and drain the consumed prefix
            // exactly once at the end. Draining 480 samples off the *front* every
            // frame instead is O(n²) — fine for the small live chunks, but the
            // offline `clean_wav_file` pass hands us the whole recording in one
            // call, where that quadratic blowup turned a 30-min file into a ~45-min
            // "enhancing…" wait. A cursor keeps this linear.
            let mut out = Vec::with_capacity(self.pending.len());
            let mut frame = [0.0f32; FRAME_SIZE];
            let mut frame_out = [0.0f32; FRAME_SIZE];
            let mut consumed = 0usize;
            while consumed + FRAME_SIZE <= self.pending.len() {
                frame.copy_from_slice(&self.pending[consumed..consumed + FRAME_SIZE]);
                consumed += FRAME_SIZE;
                let prob = self.state.process_frame(&mut frame_out, &frame);
                if !self.discarded_first {
                    self.discarded_first = true; // drop fade-in frame
                    continue;
                }
                if self.gate {
                    // Move the smoothed gain toward the per-frame target: fast when
                    // opening (don't clip a speech onset), slower when closing.
                    let target = gate::target(prob);
                    let coeff = if target > self.gate_gain {
                        gate::ATTACK
                    } else {
                        gate::RELEASE
                    };
                    self.gate_gain += (target - self.gate_gain) * coeff;
                    for s in frame_out.iter_mut() {
                        *s *= self.gate_gain;
                    }
                }
                out.extend(frame_out.iter().map(|s| (s / 32768.0).clamp(-1.0, 1.0)));
            }
            self.pending.drain(0..consumed);
            out
        }

        fn name(&self) -> &'static str {
            "rnnoise"
        }
    }
}

#[cfg(all(test, feature = "denoise"))]
mod tests {
    use super::*;

    /// Feeding the whole signal in one call must produce exactly the same output
    /// as feeding it in arbitrary small chunks — i.e. framing is driven by the
    /// internal buffer, not by how the caller slices its input. This guards the
    /// cursor-based traversal that replaced the O(n²) front-drain.
    #[test]
    fn one_shot_matches_chunked() {
        let n = FRAME_SIZE * 50 + 123; // not frame-aligned, so a remainder is held
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();

        let mut one = RnnoiseDenoiser::new();
        let whole = one.process(&input);

        let mut streamed = RnnoiseDenoiser::new();
        let mut chunked = Vec::new();
        for chunk in input.chunks(97) {
            chunked.extend(streamed.process(chunk));
        }

        assert_eq!(whole, chunked, "output must not depend on input chunking");
        // Full 480-sample frames, minus the discarded RNNoise fade-in frame.
        assert_eq!(whole.len(), (n / FRAME_SIZE - 1) * FRAME_SIZE);
    }

    /// The residual gate must not break the framing invariant: one-shot output
    /// still equals chunked output, and the sample count matches the ungated path
    /// (the gate scales samples, it never adds or drops them).
    #[test]
    fn gated_is_chunk_independent_and_same_length() {
        let n = FRAME_SIZE * 40 + 55;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.02).sin() * 0.3).collect();

        let mut one = RnnoiseDenoiser::gated();
        let whole = one.process(&input);

        let mut streamed = RnnoiseDenoiser::gated();
        let mut chunked = Vec::new();
        for c in input.chunks(101) {
            chunked.extend(streamed.process(c));
        }
        assert_eq!(whole, chunked, "gated output must not depend on chunking");
        assert_eq!(whole.len(), (n / FRAME_SIZE - 1) * FRAME_SIZE);
    }

    /// The gate curve is bounded to [FLOOR, 1.0] and rises monotonically with the
    /// speech probability.
    #[test]
    fn gate_target_is_bounded_and_monotonic() {
        use super::rnnoise::gate;
        assert!((gate::target(0.0) - gate::FLOOR).abs() < 1e-6);
        assert!((gate::target(1.0) - 1.0).abs() < 1e-6);
        let (a, b, c) = (gate::target(0.2), gate::target(0.45), gate::target(0.8));
        assert!(a <= b && b <= c, "gate target must be monotonic in prob");
        assert!((gate::FLOOR..=1.0).contains(&gate::target(0.5)));
    }
}

/// Build a denoiser: RNNoise when `enabled` and the feature is compiled in,
/// otherwise a pass-through. When `gate` is set, RNNoise is followed by the
/// speech-probability residual gate — use it for the microphone (a single
/// speaker) and leave it off for the far-end (music / several remote speakers a
/// single-speaker gate would damage).
pub fn make_gated(enabled: bool, gate: bool) -> Box<dyn Denoiser> {
    #[cfg(feature = "denoise")]
    {
        if enabled {
            return if gate {
                Box::new(RnnoiseDenoiser::gated())
            } else {
                Box::new(RnnoiseDenoiser::new())
            };
        }
    }
    #[cfg(not(feature = "denoise"))]
    {
        let _ = (enabled, gate);
    }
    Box::new(PassThrough)
}

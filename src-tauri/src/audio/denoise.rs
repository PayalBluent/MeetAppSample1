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

    /// RNNoise-backed suppressor. Buffers input into 480-sample frames and
    /// discards the very first output frame (RNNoise fade-in artifact).
    pub struct RnnoiseDenoiser {
        state: Box<DenoiseState<'static>>,
        /// Pending input, already scaled to i16 range.
        pending: Vec<f32>,
        discarded_first: bool,
    }

    impl RnnoiseDenoiser {
        pub fn new() -> Self {
            Self {
                state: DenoiseState::new(),
                pending: Vec::with_capacity(FRAME_SIZE * 4),
                discarded_first: false,
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

            let mut out = Vec::with_capacity(self.pending.len());
            let mut frame_out = [0.0f32; FRAME_SIZE];
            while self.pending.len() >= FRAME_SIZE {
                let frame: Vec<f32> = self.pending.drain(0..FRAME_SIZE).collect();
                self.state.process_frame(&mut frame_out, &frame);
                if !self.discarded_first {
                    self.discarded_first = true; // drop fade-in frame
                    continue;
                }
                out.extend(frame_out.iter().map(|s| (s / 32768.0).clamp(-1.0, 1.0)));
            }
            out
        }

        fn name(&self) -> &'static str {
            "rnnoise"
        }
    }
}

/// Build a denoiser: RNNoise when `enabled` and the feature is compiled in,
/// otherwise a pass-through.
pub fn make(enabled: bool) -> Box<dyn Denoiser> {
    #[cfg(feature = "denoise")]
    {
        if enabled {
            return Box::new(RnnoiseDenoiser::new());
        }
    }
    #[cfg(not(feature = "denoise"))]
    {
        let _ = enabled;
    }
    Box::new(PassThrough)
}

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

#[cfg(feature = "denoise")]
mod rnnoise {
    use super::{Denoiser, FRAME_SIZE};
    use nnnoiseless::DenoiseState;

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

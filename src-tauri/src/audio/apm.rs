//! WebRTC Audio Processing (APM) for the microphone: acoustic echo cancellation
//! (AEC3), automatic gain control (AGC2), and a high-pass filter.
//!
//! This is **additive and optional**. It's compiled only with the `apm` Cargo
//! feature, and even then the pipeline uses it only if [`Apm::new`] succeeds; on
//! any failure (or with the feature off) the microphone path is left exactly as
//! it was and the existing RNNoise noise-suppression pass is the fallback. RNNoise
//! is never touched by this module.
//!
//! AEC needs the far-end (system) audio as a reference: feed it with
//! [`Apm::push_render`], and clean the mic with [`Apm::process_capture`]. Both
//! sides are 48 kHz mono and are internally re-framed to WebRTC's 10 ms frames.

#[cfg(feature = "apm")]
pub use imp::Apm;
#[cfg(not(feature = "apm"))]
pub use stub::Apm;

#[cfg(feature = "apm")]
mod imp {
    use webrtc_audio_processing::config::{
        AdaptiveDigital, Config, EchoCanceller, FixedDigital, GainController, GainController2,
        HighPassFilter,
    };
    use webrtc_audio_processing::Processor;

    /// A WebRTC APM instance for one mono mic + mono reference, re-framing 48 kHz
    /// audio into the processor's native 10 ms frames.
    pub struct Apm {
        processor: Processor,
        /// Samples per 10 ms frame (480 @ 48 kHz).
        frame: usize,
        /// Reusable single-channel scratch buffer (`Vec<Vec<f32>>`, one channel).
        scratch: Vec<Vec<f32>>,
        /// Mic samples awaiting a full frame.
        capture_buf: Vec<f32>,
        /// Reference (system) samples awaiting a full frame.
        render_buf: Vec<f32>,
    }

    impl Apm {
        /// Build an APM tuned for a meeting mic: AEC3 + AGC2 (adaptive digital) +
        /// high-pass filter. Noise suppression is left to the existing RNNoise
        /// pass, so it is deliberately disabled here. Returns `None` if the
        /// processor can't initialize — the caller then runs without APM.
        pub fn new() -> Option<Apm> {
            let processor = Processor::new(48_000).ok()?;
            let config = Config {
                echo_canceller: Some(EchoCanceller::Full { stream_delay_ms: None }),
                high_pass_filter: Some(HighPassFilter { apply_in_full_band: true }),
                gain_controller: Some(GainController::GainController2(GainController2 {
                    input_volume_controller_enabled: false,
                    adaptive_digital: Some(AdaptiveDigital::default()),
                    fixed_digital: FixedDigital::default(),
                })),
                // RNNoise handles noise suppression downstream — don't double up.
                noise_suppression: None,
                ..Default::default()
            };
            processor.set_config(config);
            let frame = processor.num_samples_per_frame();
            Some(Apm {
                processor,
                frame,
                scratch: vec![vec![0.0f32; frame]],
                capture_buf: Vec::new(),
                render_buf: Vec::new(),
            })
        }

        /// Feed far-end (system) reference audio, 48 kHz mono. Analyzed for echo
        /// cancellation; the samples themselves are not returned or modified.
        pub fn push_render(&mut self, mono48: &[f32]) {
            self.render_buf.extend_from_slice(mono48);
            while self.render_buf.len() >= self.frame {
                self.scratch[0].clear();
                self.scratch[0].extend_from_slice(&self.render_buf[..self.frame]);
                self.render_buf.drain(..self.frame);
                let _ = self.processor.analyze_render_frame(&mut self.scratch);
            }
        }

        /// Process near-end (mic) audio, 48 kHz mono, through AEC/AGC/HPF and
        /// return the cleaned samples. Partial frames are buffered across calls,
        /// so the returned length may lag the input by up to one 10 ms frame.
        pub fn process_capture(&mut self, mono48: &[f32]) -> Vec<f32> {
            self.capture_buf.extend_from_slice(mono48);
            let mut out = Vec::with_capacity(self.capture_buf.len());
            while self.capture_buf.len() >= self.frame {
                self.scratch[0].clear();
                self.scratch[0].extend_from_slice(&self.capture_buf[..self.frame]);
                self.capture_buf.drain(..self.frame);
                // On error, fall through with the unmodified frame rather than drop audio.
                let _ = self.processor.process_capture_frame(&mut self.scratch);
                out.extend_from_slice(&self.scratch[0]);
            }
            out
        }
    }
}

#[cfg(not(feature = "apm"))]
mod stub {
    /// No-op stand-in when the `apm` feature is off. [`Apm::new`] returns `None`,
    /// so the pipeline never constructs one and the mic path is unchanged.
    pub struct Apm;

    impl Apm {
        pub fn new() -> Option<Apm> {
            None
        }
        pub fn push_render(&mut self, _mono48: &[f32]) {}
        pub fn process_capture(&mut self, mono48: &[f32]) -> Vec<f32> {
            mono48.to_vec()
        }
    }
}

//! Voice activity detection.
//!
//! Speech is detected from RNNoise's trained voice-activity probability (see
//! [`crate::audio::denoise::SpeechDetector`]) — *not* raw loudness — gated by a
//! small energy floor and smoothed with a short hangover so brief pauses between
//! words don't chop a phrase into many segments. Using the neural probability is
//! what makes this fire on speech and stay quiet for music, typing, keyboard
//! clatter, and background hum. Real diarization is still left to the cloud
//! transcription step.

/// Stateful VAD for a single audio stream.
pub struct Vad {
    /// Speech probability (0..1) at/above which a block counts as speech.
    prob_threshold: f32,
    /// RMS floor below which we never report speech regardless of probability —
    /// guards against a spurious high-probability frame on near-silence.
    energy_floor: f32,
    /// How many consecutive non-speech blocks to still report as speech (tail).
    hangover: u32,
    remaining: u32,
}

impl Vad {
    pub fn new() -> Self {
        Vad {
            prob_threshold: 0.5,
            energy_floor: 0.003,
            hangover: 20,
            remaining: 0,
        }
    }

    /// Update with the latest block's speech probability (from RNNoise) and its
    /// RMS energy; returns whether the stream is currently considered *speaking*.
    pub fn update(&mut self, speech_prob: f32, rms: f32) -> bool {
        if speech_prob >= self.prob_threshold && rms >= self.energy_floor {
            self.remaining = self.hangover;
            true
        } else if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_speech_not_loudness() {
        let mut vad = Vad::new();
        // Loud, but not speech (low probability) → ignored. This is the whole point.
        assert!(!vad.update(0.05, 0.4), "loud non-speech is not speech");
        // Speech-shaped probability but essentially silent → ignored.
        assert!(!vad.update(0.9, 0.0001), "speech prob on silence is not speech");
        // Genuine speech: high probability and real energy.
        assert!(vad.update(0.9, 0.2), "loud speech is detected");
        // Stays active through the hangover window, then falls back to silence.
        for _ in 0..20 {
            assert!(vad.update(0.0, 0.2));
        }
        assert!(!vad.update(0.0, 0.2), "silence resumes after hangover");
    }
}

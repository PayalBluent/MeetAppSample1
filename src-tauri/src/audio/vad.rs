//! Voice activity detection.
//!
//! Deliberately simple: an energy (RMS) threshold with a short hangover so brief
//! dips between words don't chop a phrase into many segments. This is enough to
//! label speech vs. silence per stream; real diarization is left to the cloud
//! transcription step.

/// Stateful VAD for a single audio stream.
pub struct Vad {
    /// RMS above which a block counts as speech (~-40 dBFS).
    threshold: f32,
    /// How many consecutive quiet blocks to still report as speech.
    hangover: u32,
    remaining: u32,
}

impl Vad {
    pub fn new() -> Self {
        Vad {
            threshold: 0.01,
            hangover: 10,
            remaining: 0,
        }
    }

    /// Update with the RMS of the latest block; returns whether the stream is
    /// currently considered active (speaking).
    pub fn update(&mut self, rms: f32) -> bool {
        if rms >= self.threshold {
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
    fn detects_speech_with_hangover() {
        let mut vad = Vad::new();
        assert!(!vad.update(0.0), "silence is not speech");
        assert!(vad.update(0.2), "loud block is speech");
        // Stays active through the hangover window, then falls back to silence.
        for _ in 0..10 {
            assert!(vad.update(0.0));
        }
        assert!(!vad.update(0.0), "silence resumes after hangover");
    }
}

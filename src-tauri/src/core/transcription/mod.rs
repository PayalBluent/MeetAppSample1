use std::sync::atomic::{AtomicUsize, Ordering};

use crate::models::TranscriptSegment;

#[cfg(feature = "whisper")]
pub mod whisper;

/// A speech-to-text backend. Implementations must be `Send + Sync` so the
/// recorder can drive them from its capture thread.
///
/// The interface is intentionally simple and pull-based: the recorder calls
/// [`Transcriber::next`] periodically, handing over the most recent audio buffer
/// (empty in the simulated backend) and the current elapsed time.
pub trait Transcriber: Send + Sync {
    /// Human-readable model/provider label, surfaced in the UI.
    fn model_name(&self) -> String;

    /// Produce the next transcript segment, if one is ready.
    fn next(&self, elapsed_ms: u64, pcm: &[f32], sample_rate: u32) -> Option<TranscriptSegment>;

    /// Reset any internal cursor between sessions.
    fn reset(&self) {}
}

fn seg(speaker: &str, text: &str, start_ms: u64, end_ms: u64) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("seg_{}", uuid::Uuid::new_v4().simple()),
        speaker: speaker.to_string(),
        text: text.to_string(),
        start_ms,
        end_ms,
        confidence: Some(0.94),
    }
}

/// Default backend: a realistic scripted transcript. This keeps the shipped app
/// fully functional end-to-end without requiring a multi-gigabyte model download
/// or a C++ toolchain. Enable the `whisper` feature for real on-device STT.
pub struct SimulatedTranscriber {
    cursor: AtomicUsize,
}

impl SimulatedTranscriber {
    pub fn new() -> Self {
        Self {
            cursor: AtomicUsize::new(0),
        }
    }
}

impl Default for SimulatedTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

const LINES: &[(&str, &str)] = &[
    ("You", "Okay, let's get started — I think everyone's here now."),
    ("Priya Nair", "Great. First item is the release timeline for next week."),
    ("Marcus Lee", "I'll have the build ready by Wednesday so QA has two full days."),
    ("You", "Perfect. We'll need sign-off from design before we ship."),
    ("Dana Whitfield", "Design is good to go, I'll post the final specs in the channel."),
    ("Priya Nair", "Let's decide on the rollout — staged or all at once?"),
    ("You", "Staged. Ten percent first, then ramp if metrics look healthy."),
    ("Marcus Lee", "Agreed. I'll set up the feature flag for the staged rollout."),
];

impl Transcriber for SimulatedTranscriber {
    fn model_name(&self) -> String {
        "local · simulated".to_string()
    }

    fn next(&self, elapsed_ms: u64, _pcm: &[f32], _sample_rate: u32) -> Option<TranscriptSegment> {
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed);
        let (speaker, text) = LINES[idx % LINES.len()];
        Some(seg(speaker, text, elapsed_ms, elapsed_ms + 3_000))
    }

    fn reset(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }
}

/// The full built-in transcript as a one-shot list, used as the offline fallback
/// when no cloud speech-to-text is configured (see [`crate::core::recorder`]).
/// Mirrors the browser mock so Transcribe mode produces the same result with or
/// without the native backend.
pub fn simulated_segments() -> Vec<TranscriptSegment> {
    LINES
        .iter()
        .enumerate()
        .map(|(i, (speaker, text))| {
            let start = i as u64 * 4_000;
            seg(speaker, text, start, start + 3_500)
        })
        .collect()
}

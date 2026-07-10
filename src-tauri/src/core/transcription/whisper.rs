//! On-device transcription via whisper.cpp (whisper-rs).
//!
//! Enabled with the `whisper` feature. Requires a GGML/GGUF model on disk
//! (set `MEETAPP_WHISPER_MODEL` to its path) and a C/C++ toolchain + CMake to
//! build whisper.cpp. See the README for setup.

use std::path::PathBuf;
use std::sync::Mutex;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{seg, Transcriber};
use crate::models::TranscriptSegment;

pub struct WhisperTranscriber {
    ctx: WhisperContext,
    model_label: String,
    /// Monotonic counter so we only emit newly-produced text.
    last_len: Mutex<usize>,
}

impl WhisperTranscriber {
    /// Load a model from `MEETAPP_WHISPER_MODEL` (or the provided path).
    pub fn load(model_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = model_path
            .or_else(|| std::env::var_os("MEETAPP_WHISPER_MODEL").map(PathBuf::from))
            .ok_or_else(|| anyhow::anyhow!("no whisper model configured"))?;
        let label = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "whisper".into());

        let ctx = WhisperContext::new_with_params(
            &path.to_string_lossy(),
            WhisperContextParameters::default(),
        )?;
        Ok(Self {
            ctx,
            model_label: format!("whisper · {label}"),
            last_len: Mutex::new(0),
        })
    }
}

/// Downmix to mono and linearly resample to 16 kHz — the format whisper expects.
fn to_mono_16k(pcm: &[f32], sample_rate: u32, channels: u16) -> Vec<f32> {
    let mono: Vec<f32> = if channels <= 1 {
        pcm.to_vec()
    } else {
        pcm.chunks(channels as usize)
            .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
            .collect()
    };
    if sample_rate == 16_000 || mono.is_empty() {
        return mono;
    }
    let ratio = 16_000f32 / sample_rate as f32;
    let out_len = (mono.len() as f32 * ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f32 / ratio;
            let i0 = src.floor() as usize;
            let i1 = (i0 + 1).min(mono.len() - 1);
            let frac = src - i0 as f32;
            mono[i0] * (1.0 - frac) + mono[i1] * frac
        })
        .collect()
}

impl Transcriber for WhisperTranscriber {
    fn model_name(&self) -> String {
        self.model_label.clone()
    }

    fn next(&self, elapsed_ms: u64, pcm: &[f32], sample_rate: u32) -> Option<TranscriptSegment> {
        // Need at least ~2s of audio to produce a stable segment.
        if pcm.len() < sample_rate as usize {
            return None;
        }
        let samples = to_mono_16k(pcm, sample_rate, 1);

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(num_threads());
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = match self.ctx.create_state() {
            Ok(s) => s,
            Err(_) => return None,
        };
        if state.full(params, &samples).is_err() {
            return None;
        }

        let n = state.full_n_segments().unwrap_or(0);
        let mut last = self.last_len.lock().unwrap();
        if (n as usize) <= *last {
            return None;
        }
        let text = state.full_get_segment_text(n - 1).ok()?;
        *last = n as usize;
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(seg("Speaker", &text, elapsed_ms, elapsed_ms + 3_000))
    }

    fn reset(&self) {
        *self.last_len.lock().unwrap() = 0;
    }
}

fn num_threads() -> std::os::raw::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8) as std::os::raw::c_int)
        .unwrap_or(4)
}

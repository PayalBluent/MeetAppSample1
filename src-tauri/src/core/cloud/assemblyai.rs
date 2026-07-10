//! AssemblyAI transcription (pre-recorded audio, with speaker diarization).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::map_ureq;
use crate::error::{AppError, AppResult};
use crate::models::{Participant, TranscriptSegment};

const BASE: &str = "https://api.assemblyai.com/v2";

pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
    pub participants: Vec<Participant>,
}

/// Upload a local audio file, request a diarized transcript, and poll to completion.
pub fn transcribe_file(path: &Path, api_key: &str) -> AppResult<Transcript> {
    let bytes = std::fs::read(path)?;
    tracing::info!("AssemblyAI: uploading {} bytes", bytes.len());

    // 1) Upload the raw audio → temporary URL.
    let up: Value = ureq::post(&format!("{BASE}/upload"))
        .set("authorization", api_key)
        .send_bytes(&bytes)
        .map_err(map_ureq)?
        .into_json()
        .map_err(AppError::Io)?;
    let upload_url = up["upload_url"]
        .as_str()
        .ok_or_else(|| AppError::Transcription("AssemblyAI: no upload_url returned".into()))?;

    // 2) Create the transcription job.
    let create: Value = ureq::post(&format!("{BASE}/transcript"))
        .set("authorization", api_key)
        .send_json(serde_json::json!({
            "audio_url": upload_url,
            "speaker_labels": true,
            "punctuate": true,
            "format_text": true,
        }))
        .map_err(map_ureq)?
        .into_json()
        .map_err(AppError::Io)?;
    let id = create["id"]
        .as_str()
        .ok_or_else(|| AppError::Transcription("AssemblyAI: no transcript id".into()))?
        .to_string();

    // 3) Poll until done — bounded so a stuck job can't block the worker forever.
    let poll_url = format!("{BASE}/transcript/{id}");
    const MAX_ATTEMPTS: usize = 200; // ~10 minutes at 3s between polls
    for _ in 0..MAX_ATTEMPTS {
        std::thread::sleep(Duration::from_secs(3));
        let t: Value = ureq::get(&poll_url)
            .set("authorization", api_key)
            .call()
            .map_err(map_ureq)?
            .into_json()
            .map_err(AppError::Io)?;
        match t["status"].as_str().unwrap_or("") {
            "completed" => return Ok(parse(&t)),
            "error" => {
                return Err(AppError::Transcription(format!(
                    "AssemblyAI: {}",
                    t["error"].as_str().unwrap_or("unknown error")
                )))
            }
            other => tracing::debug!("AssemblyAI status: {other}"),
        }
    }
    Err(AppError::Transcription(
        "AssemblyAI: transcription timed out".into(),
    ))
}

fn parse(t: &Value) -> Transcript {
    let mut segments = Vec::new();
    let mut talk: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;

    if let Some(utterances) = t["utterances"].as_array() {
        for u in utterances {
            let text = u["text"].as_str().unwrap_or("").trim().to_string();
            if text.is_empty() {
                continue;
            }
            let speaker = format!("Speaker {}", u["speaker"].as_str().unwrap_or("A"));
            let start = u["start"].as_u64().unwrap_or(0);
            let end = u["end"].as_u64().unwrap_or(start);
            let dur = end.saturating_sub(start);
            *talk.entry(speaker.clone()).or_insert(0) += dur;
            total += dur;
            segments.push(TranscriptSegment {
                id: seg_id(),
                speaker,
                text,
                start_ms: start,
                end_ms: end,
                confidence: u["confidence"].as_f64().map(|c| c as f32),
            });
        }
    }

    // Fallback: no diarized utterances → use the flat transcript text.
    if segments.is_empty() {
        if let Some(text) = t["text"].as_str() {
            if !text.trim().is_empty() {
                let end = t["audio_duration"].as_u64().unwrap_or(0) * 1000;
                segments.push(TranscriptSegment {
                    id: seg_id(),
                    speaker: "Speaker".into(),
                    text: text.trim().to_string(),
                    start_ms: 0,
                    end_ms: end,
                    confidence: t["confidence"].as_f64().map(|c| c as f32),
                });
            }
        }
    }

    let participants = talk
        .into_iter()
        .map(|(name, ms)| Participant {
            id: format!("p_{}", uuid::Uuid::new_v4().simple()),
            name,
            talk_ratio: if total > 0 {
                Some(ms as f32 / total as f32)
            } else {
                None
            },
        })
        .collect();

    Transcript {
        segments,
        participants,
    }
}

fn seg_id() -> String {
    format!("seg_{}", uuid::Uuid::new_v4().simple())
}

//! Groq summarization via the OpenAI-compatible chat-completions API (JSON mode).

use chrono::Utc;
use serde_json::Value;

use super::map_ureq;
use crate::error::{AppError, AppResult};
use crate::models::{ActionItem, MeetingSummary, TranscriptSegment};

const ENDPOINT: &str = "https://api.groq.com/openai/v1/chat/completions";

const SYSTEM_PROMPT: &str = "You are an expert meeting assistant. Read the transcript and reply with STRICT JSON only (no markdown, no prose) in exactly this shape: {\"tldr\": string, \"keyPoints\": string[], \"decisions\": string[], \"actionItems\": [{\"text\": string, \"assignee\": string|null}]}. Keep it concise and factual, grounded only in the transcript.";

pub fn summarize(
    title: &str,
    segments: &[TranscriptSegment],
    api_key: &str,
    model: &str,
) -> AppResult<(MeetingSummary, Vec<ActionItem>)> {
    let transcript: String = segments
        .iter()
        .map(|s| format!("{}: {}", s.speaker, s.text))
        .collect::<Vec<_>>()
        .join("\n");
    if transcript.trim().is_empty() {
        return Err(AppError::Other("Empty transcript.".into()));
    }

    let body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": format!("Meeting title: {title}\n\nTranscript:\n{transcript}") },
        ],
    });

    let resp: Value = ureq::post(ENDPOINT)
        .set("authorization", &format!("Bearer {api_key}"))
        .send_json(body)
        .map_err(map_ureq)?
        .into_json()
        .map_err(AppError::Io)?;

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| AppError::Other("Groq returned no content".into()))?;
    let parsed: Value = serde_json::from_str(content)
        .map_err(|e| AppError::Other(format!("Groq returned invalid JSON: {e}")))?;

    let summary = MeetingSummary {
        tldr: parsed["tldr"].as_str().unwrap_or_default().to_string(),
        key_points: str_array(&parsed["keyPoints"]),
        decisions: str_array(&parsed["decisions"]),
        generated_at: Utc::now(),
        model: format!("groq · {model}"),
    };

    let action_items = parsed["actionItems"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|it| {
                    let text = it["text"].as_str().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        return None;
                    }
                    Some(ActionItem {
                        id: format!("a_{}", uuid::Uuid::new_v4().simple()),
                        text,
                        done: false,
                        assignee: it["assignee"]
                            .as_str()
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty() && *s != "null")
                            .map(String::from),
                        due_date: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((summary, action_items))
}

fn str_array(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

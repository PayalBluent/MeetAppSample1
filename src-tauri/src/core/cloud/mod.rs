//! Cloud AI backends.
//!
//! Transcription via **AssemblyAI** (upload → transcribe → poll, with speaker
//! diarization) and summarization via **Groq** (OpenAI-compatible chat
//! completions, JSON mode). Both are reached over plain HTTPS with `ureq`.
//!
//! Keys come from Settings, or the `ASSEMBLYAI_API_KEY` / `GROQ_API_KEY` env
//! vars when a field is blank. Summarization gracefully falls back to the local
//! heuristic summarizer when no Groq key is configured.

pub mod assemblyai;
pub mod groq;

use std::path::Path;

use tauri::AppHandle;

use crate::core::ai::{HeuristicSummarizer, Summarizer};
use crate::error::{AppError, AppResult};
use crate::events::Events;
use crate::models::{Meeting, MeetingStatus};
use crate::state::AppState;

/// Resolve an API key: prefer the settings value, else the environment variable.
pub fn resolve_key(setting: &str, env_var: &str) -> Option<String> {
    let s = setting.trim();
    if !s.is_empty() {
        return Some(s.to_string());
    }
    std::env::var(env_var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Map a `ureq` error (incl. non-2xx responses) to a user-facing message.
pub(crate) fn map_ureq(e: ureq::Error) -> AppError {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let snippet: String = body.chars().take(400).collect();
            AppError::Transcription(format!("HTTP {code}: {snippet}"))
        }
        other => AppError::Transcription(other.to_string()),
    }
}

/// Transcribe a meeting's audio via AssemblyAI and update state + emit events.
/// `delete_audio_after` removes the WAV afterwards (Transcribe-only mode, which
/// must not retain audio).
pub fn run_transcription(
    app: &AppHandle,
    state: &AppState,
    meeting_id: &str,
    delete_audio_after: bool,
) -> AppResult<Meeting> {
    let (audio_path, key) = {
        let meetings = state.meetings.read();
        let m = meetings
            .get(meeting_id)
            .ok_or_else(|| AppError::MeetingNotFound(meeting_id.into()))?;
        let path = m
            .audio_path
            .clone()
            .ok_or_else(|| AppError::Other("This meeting has no saved audio to transcribe.".into()))?;
        let settings = state.settings.read();
        let key = resolve_key(&settings.assemblyai_api_key, "ASSEMBLYAI_API_KEY")
            .ok_or_else(|| AppError::Other("Add your AssemblyAI API key in Settings to transcribe.".into()))?;
        (path, key)
    };

    set_status(app, state, meeting_id, MeetingStatus::Processing);

    match assemblyai::transcribe_file(Path::new(&audio_path), &key) {
        Ok(tr) => {
            let updated = {
                let mut meetings = state.meetings.write();
                let m = meetings
                    .get_mut(meeting_id)
                    .ok_or_else(|| AppError::MeetingNotFound(meeting_id.into()))?;
                m.transcript = tr.segments;
                if !tr.participants.is_empty() {
                    m.participants = tr.participants;
                }
                m.status = MeetingStatus::Ready;
                if delete_audio_after {
                    let _ = std::fs::remove_file(&audio_path);
                    m.audio_path = None;
                    m.has_audio = false;
                }
                m.clone()
            };
            Events::updated(app, &updated);
            Ok(updated)
        }
        Err(e) => {
            set_status(app, state, meeting_id, MeetingStatus::Failed);
            Err(e)
        }
    }
}

/// Summarize a meeting's transcript via Groq (or the local heuristic fallback).
pub fn run_summarization(app: &AppHandle, state: &AppState, meeting_id: &str) -> AppResult<Meeting> {
    let meeting = {
        let meetings = state.meetings.read();
        meetings
            .get(meeting_id)
            .cloned()
            .ok_or_else(|| AppError::MeetingNotFound(meeting_id.into()))?
    };
    if meeting.transcript.is_empty() {
        return Err(AppError::Other(
            "Transcribe the meeting first, then summarize.".into(),
        ));
    }

    let (groq_key, model) = {
        let s = state.settings.read();
        (
            resolve_key(&s.groq_api_key, "GROQ_API_KEY"),
            s.groq_model.clone(),
        )
    };

    let (summary, action_items) = match groq_key {
        Some(key) => groq::summarize(&meeting.title, &meeting.transcript, &key, &model)?,
        None => {
            let h = HeuristicSummarizer;
            (h.summarize(&meeting), h.action_items(&meeting))
        }
    };

    let updated = {
        let mut meetings = state.meetings.write();
        let m = meetings
            .get_mut(meeting_id)
            .ok_or_else(|| AppError::MeetingNotFound(meeting_id.into()))?;
        m.summary = Some(summary);
        m.action_items = action_items;
        m.status = MeetingStatus::Ready;
        m.clone()
    };
    Events::updated(app, &updated);
    Ok(updated)
}

fn set_status(app: &AppHandle, state: &AppState, id: &str, status: MeetingStatus) {
    let updated = {
        let mut meetings = state.meetings.write();
        meetings.get_mut(id).map(|m| {
            m.status = status;
            m.clone()
        })
    };
    if let Some(m) = updated {
        Events::updated(app, &m);
    }
}

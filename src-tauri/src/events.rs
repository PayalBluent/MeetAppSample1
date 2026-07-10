use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::models::{DetectedMeeting, Meeting, RecorderStatus, TranscriptSegment};

/// Canonical event names. These MUST match the keys in the frontend `AppEvents`
/// type (`src/types/index.ts`).
pub mod names {
    pub const MEETING_DETECTED: &str = "meeting://detected";
    pub const MEETING_ENDED: &str = "meeting://ended";
    pub const MEETING_UPDATED: &str = "meeting://updated";
    pub const RECORDER_STATUS: &str = "recorder://status";
    pub const RECORDER_TRANSCRIPT: &str = "recorder://transcript";
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEvent {
    pub meeting_id: String,
    pub segment: TranscriptSegment,
}

#[derive(Serialize, Clone)]
pub struct MeetingEndedEvent {
    pub id: String,
}

/// Typed emit helpers so producers never mistype an event name or payload shape.
pub struct Events;

impl Events {
    pub fn detected(app: &AppHandle, payload: &DetectedMeeting) {
        let _ = app.emit(names::MEETING_DETECTED, payload);
    }

    pub fn ended(app: &AppHandle, id: impl Into<String>) {
        let _ = app.emit(names::MEETING_ENDED, MeetingEndedEvent { id: id.into() });
    }

    pub fn updated(app: &AppHandle, meeting: &Meeting) {
        let _ = app.emit(names::MEETING_UPDATED, meeting);
    }

    pub fn status(app: &AppHandle, status: &RecorderStatus) {
        let _ = app.emit(names::RECORDER_STATUS, status);
    }

    pub fn transcript(app: &AppHandle, meeting_id: impl Into<String>, segment: TranscriptSegment) {
        let _ = app.emit(
            names::RECORDER_TRANSCRIPT,
            TranscriptEvent {
                meeting_id: meeting_id.into(),
                segment,
            },
        );
    }
}

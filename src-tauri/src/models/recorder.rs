use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::meeting::{CaptureMode, MeetingPlatform};

/// Lifecycle of the recorder subsystem. Mirrors the frontend `RecorderState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecorderState {
    Idle,
    Armed,
    Detecting,
    Recording,
    Processing,
    Error,
}

/// Aggregate recorder status streamed to the UI via `recorder://status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: RecorderState,
    pub mode: CaptureMode,
    #[serde(default)]
    pub active_meeting_id: Option<String>,
    pub elapsed_sec: u64,
    pub mic_level: f32,
    pub system_level: f32,
    #[serde(default)]
    pub message: Option<String>,
}

impl Default for RecorderStatus {
    fn default() -> Self {
        RecorderStatus {
            state: RecorderState::Idle,
            mode: CaptureMode::Off,
            active_meeting_id: None,
            elapsed_sec: 0,
            mic_level: 0.0,
            system_level: 0.0,
            message: None,
        }
    }
}

/// A meeting the detector currently believes is in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedMeeting {
    pub id: String,
    pub platform: MeetingPlatform,
    pub title: String,
    pub process_name: String,
    pub detected_at: DateTime<Utc>,
    pub capturing: bool,
}

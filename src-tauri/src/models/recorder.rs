use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::meeting::{CaptureMode, MeetingPlatform};

/// Default capture volume (input gain) multiplier. A mild boost so recordings —
/// especially quiet microphones — come out audible without the user touching the
/// volume control. Applied to captured samples before they're written.
pub const DEFAULT_INPUT_GAIN: f32 = 1.5;

/// Highest gain the volume control allows. Beyond this, loud sources clip harshly
/// with little perceived benefit.
pub const MAX_INPUT_GAIN: f32 = 3.0;

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
    /// Capture volume multiplier the pipeline applies to recorded audio. `1.0` is
    /// unity; the default is a mild boost. Adjustable live via the volume control.
    #[serde(default = "default_gain")]
    pub input_gain: f32,
    /// `true` once capture is live (first audio packet processed). Stays `false`
    /// during the device-setup window right after recording starts, so the UI can
    /// show a "starting…" state and only enable transcription once audio flows.
    #[serde(default)]
    pub audio_ready: bool,
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
            input_gain: DEFAULT_INPUT_GAIN,
            audio_ready: false,
            message: None,
        }
    }
}

fn default_gain() -> f32 {
    DEFAULT_INPUT_GAIN
}

/// Result of probing the Windows shared-mode audio engine, streamed to the UI so
/// it can offer the "Repair audio" action when shared mode is impaired.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioHealth {
    /// False on non-Windows platforms (the shared-mode APO issue is Windows-only).
    pub supported: bool,
    /// Shared-mode `IAudioClient::Initialize` succeeds — the normal, healthy path
    /// every recorder (and conferencing app) uses.
    pub shared_ok: bool,
    /// Exclusive-mode capture works — the mic can still be recorded (with the
    /// meeting-app conflict trade-off) even when shared mode is broken.
    pub exclusive_ok: bool,
    /// Shared mode is impaired but the machine is otherwise healthy — recommend the
    /// one-click repair (disable the broken enhancement + restart Windows Audio).
    pub needs_repair: bool,
    /// Human-readable one-line summary for the UI.
    pub detail: String,
}

impl Default for AudioHealth {
    fn default() -> Self {
        AudioHealth {
            supported: false,
            shared_ok: false,
            exclusive_ok: false,
            needs_repair: false,
            detail: String::new(),
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

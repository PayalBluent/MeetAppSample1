use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Meeting platforms the detector recognises. Serializes to the same strings the
/// frontend expects (see `src/types/index.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingPlatform {
    GoogleMeet,
    Zoom,
    Teams,
    Discord,
    Slack,
    Webex,
    Unknown,
}

impl MeetingPlatform {
    pub fn label(&self) -> &'static str {
        match self {
            MeetingPlatform::GoogleMeet => "Google Meet",
            MeetingPlatform::Zoom => "Zoom",
            MeetingPlatform::Teams => "Microsoft Teams",
            MeetingPlatform::Discord => "Discord",
            MeetingPlatform::Slack => "Slack Huddle",
            MeetingPlatform::Webex => "Webex",
            MeetingPlatform::Unknown => "Recording",
        }
    }
}

/// What the AI Note Taker is set to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureMode {
    Off,
    Transcribe,
    Record,
    RecordVideo,
}

impl CaptureMode {
    /// Whether this mode persists audio to disk.
    pub fn saves_audio(&self) -> bool {
        matches!(self, CaptureMode::Record | CaptureMode::RecordVideo)
    }
    /// Whether this mode captures screen video.
    pub fn saves_video(&self) -> bool {
        matches!(self, CaptureMode::RecordVideo)
    }
    /// Whether this mode produces a transcript.
    pub fn transcribes(&self) -> bool {
        !matches!(self, CaptureMode::Off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingStatus {
    Live,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    pub speaker: String,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    pub id: String,
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub tldr: String,
    pub key_points: Vec<String>,
    pub decisions: Vec<String>,
    pub generated_at: DateTime<Utc>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub talk_ratio: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineKind {
    Chapter,
    Highlight,
    Action,
    Join,
    Leave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMarker {
    pub id: String,
    pub label: String,
    pub at_ms: u64,
    pub kind: TimelineKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub platform: MeetingPlatform,
    pub mode: CaptureMode,
    pub status: MeetingStatus,

    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_sec: u64,

    pub has_audio: bool,
    pub has_video: bool,

    pub is_locked: bool,
    pub is_starred: bool,
    pub is_bookmarked: bool,

    pub tags: Vec<String>,
    pub participants: Vec<Participant>,
    pub timeline: Vec<TimelineMarker>,

    pub transcript: Vec<TranscriptSegment>,
    #[serde(default)]
    pub summary: Option<MeetingSummary>,
    pub action_items: Vec<ActionItem>,

    #[serde(default)]
    pub audio_path: Option<String>,
    #[serde(default)]
    pub video_path: Option<String>,
}

impl Meeting {
    /// Create a fresh live meeting for a new capture session.
    pub fn new_live(title: impl Into<String>, platform: MeetingPlatform, mode: CaptureMode) -> Self {
        let now = Utc::now();
        Meeting {
            id: format!("mtg_{}", uuid::Uuid::new_v4().simple()),
            title: title.into(),
            platform,
            mode,
            status: MeetingStatus::Live,
            started_at: now,
            ended_at: None,
            duration_sec: 0,
            has_audio: mode.saves_audio(),
            has_video: mode.saves_video(),
            is_locked: false,
            is_starred: false,
            is_bookmarked: false,
            tags: Vec::new(),
            participants: vec![Participant {
                id: format!("p_{}", uuid::Uuid::new_v4().simple()),
                name: "You".into(),
                talk_ratio: Some(1.0),
            }],
            timeline: vec![TimelineMarker {
                id: format!("t_{}", uuid::Uuid::new_v4().simple()),
                label: "Recording started".into(),
                at_ms: 0,
                kind: TimelineKind::Join,
            }],
            transcript: Vec::new(),
            summary: None,
            action_items: Vec::new(),
            audio_path: None,
            video_path: None,
        }
    }
}

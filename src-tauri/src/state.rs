use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use chrono::{Duration, Utc};
use parking_lot::{Mutex, RwLock};

use crate::core::recorder::RecordingSession;
use crate::models::{
    ActionItem, CaptureMode, Meeting, MeetingPlatform, MeetingStatus, MeetingSummary, Participant,
    RecorderStatus, Settings, TimelineKind, TimelineMarker, TranscriptSegment, DEFAULT_INPUT_GAIN,
};

/// Shared, in-memory application state. All fields are independently locked so
/// the capture thread, detection thread, and command handlers can work
/// concurrently without a global lock.
pub struct Inner {
    pub meetings: RwLock<HashMap<String, Meeting>>,
    pub detected: RwLock<HashMap<String, crate::models::DetectedMeeting>>,
    pub settings: RwLock<Settings>,
    pub status: RwLock<RecorderStatus>,
    pub session: Mutex<Option<RecordingSession>>,
    /// Capture volume (input gain) as f32 bits, shared live with the audio
    /// pipeline. Adjusting it via `set_input_gain` takes effect mid-recording
    /// because the pipeline reads this every buffer.
    pub recording_gain: Arc<AtomicU32>,
    /// User's in-app microphone mute, toggled via `set_mic_mute`. Combined with the
    /// OS/device mic mute by the recorder loop (either one silences the mic), so a
    /// user muted inside a meeting app — which never touches the Windows endpoint —
    /// can still stop their mic from being recorded. Reset to unmuted at each
    /// recording start.
    pub manual_mic_mute: Arc<std::sync::atomic::AtomicBool>,
}

pub type AppState = Arc<Inner>;

impl Inner {
    pub fn new(save_dir: PathBuf) -> AppState {
        let settings = Settings::with_default_dir(save_dir);

        let mut meetings = HashMap::new();
        for m in seed_meetings() {
            meetings.insert(m.id.clone(), m);
        }

        Arc::new(Inner {
            meetings: RwLock::new(meetings),
            detected: RwLock::new(HashMap::new()),
            settings: RwLock::new(settings),
            status: RwLock::new(RecorderStatus::default()),
            session: Mutex::new(None),
            recording_gain: Arc::new(AtomicU32::new(DEFAULT_INPUT_GAIN.to_bits())),
            manual_mic_mute: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Snapshot of all meetings, newest first.
    pub fn meetings_sorted(&self) -> Vec<Meeting> {
        let mut list: Vec<Meeting> = self.meetings.read().values().cloned().collect();
        list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        list
    }

    /// Persist a meeting's metadata to disk so it survives an app restart. A no-op
    /// for meetings without a media file (e.g. demo seeds). Best-effort: any IO
    /// failure is logged inside the library, never surfaced, so persistence can
    /// never break a recording or a UI action.
    pub fn persist_meeting(&self, m: &Meeting) {
        let dir = self.settings.read().save_directory.clone();
        crate::core::library::save(std::path::Path::new(&dir), m);
    }

    /// Load every recording persisted in `recordings_dir` (and import any media
    /// file that has no sidecar yet) into the in-memory store. Called once at
    /// startup so past recordings are visible. Demo seeds already present keep their
    /// slots; loaded recordings have distinct ids and are merged in.
    pub fn load_persisted_recordings(&self, recordings_dir: &std::path::Path) {
        let mut meetings = self.meetings.write();
        for m in crate::core::library::load_all(recordings_dir) {
            meetings.insert(m.id.clone(), m);
        }
    }
}

fn uid(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

fn seg(speaker: &str, text: &str, start_ms: u64, end_ms: u64) -> TranscriptSegment {
    TranscriptSegment {
        id: uid("seg"),
        speaker: speaker.into(),
        text: text.into(),
        start_ms,
        end_ms,
        confidence: Some(0.95),
    }
}

/// A couple of example meetings so the app isn't empty on first launch. Real
/// detections and recordings populate the list from here on.
fn seed_meetings() -> Vec<Meeting> {
    let now = Utc::now();
    let started = now - Duration::hours(4);
    let transcript = vec![
        seg("Alex Rivera", "Thanks everyone — today is about locking the Q3 roadmap.", 2_000, 10_000),
        seg("Priya Nair", "Are we treating offline mode as a hard commitment?", 10_500, 16_000),
        seg("Alex Rivera", "Hard commitment. It's the top churn reason for field teams.", 16_500, 23_000),
        seg("Marcus Lee", "I'll spike the local sync layer this week to ground the estimate.", 23_500, 31_000),
    ];

    let roadmap = Meeting {
        id: "mtg_roadmap".into(),
        title: "Q3 Product Roadmap Sync".into(),
        platform: MeetingPlatform::GoogleMeet,
        mode: CaptureMode::Record,
        status: MeetingStatus::Ready,
        started_at: started,
        ended_at: Some(started + Duration::minutes(42)),
        duration_sec: 42 * 60 + 18,
        has_audio: true,
        has_video: false,
        is_locked: false,
        is_starred: true,
        is_bookmarked: true,
        tags: vec!["roadmap".into(), "product".into(), "q3".into()],
        participants: vec![
            Participant { id: uid("p"), name: "Alex Rivera".into(), talk_ratio: Some(0.34) },
            Participant { id: uid("p"), name: "Priya Nair".into(), talk_ratio: Some(0.28) },
            Participant { id: uid("p"), name: "Marcus Lee".into(), talk_ratio: Some(0.22) },
            Participant { id: uid("p"), name: "Dana Whitfield".into(), talk_ratio: Some(0.16) },
        ],
        timeline: vec![
            TimelineMarker { id: uid("t"), label: "Meeting started".into(), at_ms: 0, kind: TimelineKind::Join },
            TimelineMarker { id: uid("t"), label: "Offline mode: commitment".into(), at_ms: 16_500, kind: TimelineKind::Chapter },
        ],
        summary: Some(MeetingSummary {
            tldr: "The team committed to shipping offline mode in Q3, sequenced behind a local sync layer, with a protected hardening window. Localization was deferred to Q4.".into(),
            key_points: vec![
                "Offline mode is a hard Q3 commitment".into(),
                "Sync layer lands first (~3 weeks)".into(),
                "Two-week hardening window is non-negotiable".into(),
            ],
            decisions: vec!["Ship offline mode in Q3".into(), "Defer localization to Q4".into()],
            generated_at: started + Duration::minutes(43),
            model: "local · summarizer-v1".into(),
        }),
        action_items: vec![
            ActionItem { id: uid("a"), text: "Spike the local sync layer".into(), done: false, assignee: Some("Marcus Lee".into()), due_date: None },
            ActionItem { id: uid("a"), text: "Circulate the Q3 roadmap doc".into(), done: false, assignee: Some("Priya Nair".into()), due_date: None },
        ],
        transcript,
        audio_path: None,
        video_path: None,
    };

    let clip = Meeting {
        id: "mtg_clip".into(),
        title: "meeting-clip2".into(),
        platform: MeetingPlatform::Unknown,
        mode: CaptureMode::Record,
        status: MeetingStatus::Ready,
        started_at: now - Duration::days(14),
        ended_at: Some(now - Duration::days(14) + Duration::seconds(20)),
        duration_sec: 20,
        has_audio: true,
        has_video: false,
        is_locked: true,
        is_starred: false,
        is_bookmarked: false,
        tags: vec![],
        participants: vec![Participant { id: uid("p"), name: "You".into(), talk_ratio: Some(1.0) }],
        timeline: vec![TimelineMarker { id: uid("t"), label: "Clip start".into(), at_ms: 0, kind: TimelineKind::Join }],
        summary: None,
        action_items: vec![],
        transcript: vec![seg("You", "Quick note — follow up on the vendor contract before Friday.", 500, 8_000)],
        audio_path: None,
        video_path: None,
    };

    vec![roadmap, clip]
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::Utc;
use tauri::AppHandle;

use crate::audio::{self, Recorder, SpeakerSegment};
use crate::error::{AppError, AppResult};
use crate::events::Events;
use crate::models::{
    CaptureMode, Meeting, MeetingPlatform, MeetingStatus, Participant, RecorderState, TimelineKind,
    TimelineMarker,
};
use crate::state::AppState;

/// Control handle for the active capture, stored in [`crate::state::Inner`].
pub struct RecordingSession {
    pub meeting_id: String,
    stop: Arc<AtomicBool>,
    loop_join: Option<JoinHandle<()>>,
    pub audio_path: Option<String>,
    pub video_path: Option<String>,
}

impl RecordingSession {
    fn signal_stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.loop_join.take() {
            let _ = handle.join();
        }
    }
}

/// Begin capturing. Records audio for every mode except `Off`; Transcribe mode
/// writes to a temporary file (deleted after transcription), Record / RecordVideo
/// keep their files. No transcript is produced live — that happens on demand
/// (or automatically at stop for Transcribe mode) via the cloud module.
pub fn start(
    app: &AppHandle,
    state: &AppState,
    title: String,
    platform: MeetingPlatform,
    mode: CaptureMode,
    from_detection: Option<String>,
) -> AppResult<Meeting> {
    if state.session.lock().is_some() {
        return Err(AppError::AlreadyRecording);
    }

    let mut meeting = Meeting::new_live(title, platform, mode);
    let (save_dir, capture_system_audio) = {
        let s = state.settings.read();
        (s.save_directory.clone(), s.capture_system_audio)
    };

    // Audio capture for every capturing mode: microphone (cpal) + system output
    // (WASAPI) when enabled, both into one pipeline. Transcribe mode writes to a
    // temp file that's deleted after transcription.
    let capture: Option<Recorder> = if mode != CaptureMode::Off {
        let slug = slugify(&meeting.title);
        let path = if mode == CaptureMode::Transcribe {
            let tmp = std::env::temp_dir().join("meetapp");
            let _ = std::fs::create_dir_all(&tmp);
            audio::wav_path(&tmp, &format!("{slug}-{}", meeting.id))
        } else {
            let dir = std::path::PathBuf::from(&save_dir);
            let _ = std::fs::create_dir_all(&dir);
            // Include the meeting id so distinct recordings never overwrite each
            // other (two meetings can share a title / the same "Live recording").
            audio::wav_path(&dir, &format!("{slug}-{}", meeting.id))
        };
        match Recorder::start(&path, capture_system_audio) {
            Some(h) => {
                meeting.audio_path = Some(path.to_string_lossy().replace('\\', "/"));
                Some(h)
            }
            None => {
                tracing::warn!("no audio input could be opened for this recording");
                None
            }
        }
    } else {
        None
    };

    // Screen capture for video mode (opt-in feature).
    let screen = if mode.saves_video() {
        match crate::platform::screen::start(&save_dir, &slugify(&meeting.title)) {
            Some((handle, path)) => {
                meeting.video_path = Some(path);
                Some(handle)
            }
            None => None,
        }
    } else {
        None
    };

    // `has_audio` must reflect what we actually captured, not just the mode: a
    // Record meeting whose devices failed to open has no audio to play or upload.
    // (Transcribe writes a temp file that is deleted after transcription, so it
    // always reports no retained audio — `saves_audio()` already excludes it.)
    meeting.has_audio = mode.saves_audio() && meeting.audio_path.is_some();
    let audio_unavailable = mode != CaptureMode::Off && capture.is_none();
    // The mic falls back to exclusive mode on machines whose shared audio engine is
    // impaired; that works, but seizes the device from conferencing apps — warn.
    let mic_exclusive = capture.as_ref().map(|c| c.mic_exclusive()).unwrap_or(false);

    let meeting_id = meeting.id.clone();
    state
        .meetings
        .write()
        .insert(meeting_id.clone(), meeting.clone());

    {
        let mut st = state.status.write();
        st.state = RecorderState::Recording;
        st.mode = mode;
        st.active_meeting_id = Some(meeting_id.clone());
        st.elapsed_sec = 0;
        st.message = if audio_unavailable {
            Some(
                "Couldn't open any audio input — check your microphone and Windows \
                 privacy settings. Recording without audio."
                    .into(),
            )
        } else if mic_exclusive {
            Some(
                "Recording the microphone in exclusive mode because Windows shared \
                 audio is impaired on this PC. Other apps (Zoom/Teams) may lose the \
                 microphone while recording — open Settings › Audio to repair it."
                    .into(),
            )
        } else {
            None
        };
        Events::status(app, &st);
    }
    if let Some(id) = from_detection {
        if let Some(det) = state.detected.write().get_mut(&id) {
            det.capturing = true;
        }
    }
    Events::updated(app, &meeting);

    let stop = Arc::new(AtomicBool::new(false));
    let loop_join = spawn_capture_loop(
        app.clone(),
        state.clone(),
        meeting_id.clone(),
        stop.clone(),
        capture,
        screen,
    );

    state.session.lock().replace(RecordingSession {
        meeting_id,
        stop,
        loop_join: Some(loop_join),
        audio_path: meeting.audio_path.clone(),
        video_path: meeting.video_path.clone(),
    });

    Ok(meeting)
}

fn spawn_capture_loop(
    app: AppHandle,
    state: AppState,
    meeting_id: String,
    stop: Arc<AtomicBool>,
    capture: Option<Recorder>,
    screen: Option<crate::platform::screen::ScreenHandle>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("meetapp-capture".into())
        .spawn(move || {
            let mut elapsed: u64 = 0;

            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                elapsed += 1;

                // Real input levels from the active capture (0 when none opened).
                let mic_level = capture.as_ref().map(|c| c.mic_level()).unwrap_or(0.0);
                let system_level = capture.as_ref().map(|c| c.system_level()).unwrap_or(0.0);

                if let Some(m) = state.meetings.write().get_mut(&meeting_id) {
                    m.duration_sec = elapsed;
                }
                {
                    let mut st = state.status.write();
                    st.elapsed_sec = elapsed;
                    st.mic_level = mic_level;
                    st.system_level = system_level;
                    Events::status(&app, &st);
                }
            }

            // Stop capture: this finalizes the WAV and returns the VAD speaker
            // segments. Use them for a baseline participant list ("You"/"Remote");
            // cloud transcription later refines this with real diarization.
            if let Some(c) = capture {
                let segments = c.stop();
                let participants = participants_from_segments(&segments);
                if !participants.is_empty() {
                    if let Some(m) = state.meetings.write().get_mut(&meeting_id) {
                        m.participants = participants;
                    }
                }
            }
            if let Some(s) = screen {
                s.stop();
            }
        })
        .expect("failed to spawn capture loop")
}

/// Turn VAD speaker segments into a participant list with talk-time ratios.
fn participants_from_segments(segments: &[SpeakerSegment]) -> Vec<Participant> {
    use std::collections::HashMap;

    let mut talk: HashMap<&str, u64> = HashMap::new();
    let mut total = 0u64;
    for s in segments {
        let dur = s.end_ms.saturating_sub(s.start_ms);
        *talk.entry(s.speaker).or_insert(0) += dur;
        total += dur;
    }

    // Stable order: local speaker first.
    ["You", "Remote"]
        .into_iter()
        .filter_map(|name| {
            talk.get(name).map(|&ms| Participant {
                id: format!("p_{}", uuid::Uuid::new_v4().simple()),
                name: name.to_string(),
                talk_ratio: (total > 0).then(|| ms as f32 / total as f32),
            })
        })
        .collect()
}

/// Stop the active capture. Finalizes the meeting record, then off-thread cleans
/// the audio (if enabled) and — for Transcribe mode — auto-transcribes and
/// deletes the temp audio. The returned meeting is in the `Processing` state;
/// `Ready`/`Failed` arrives via `meeting://updated`.
pub fn stop(app: &AppHandle, state: &AppState) -> AppResult<Meeting> {
    let mut session = state.session.lock().take().ok_or(AppError::NotRecording)?;
    let meeting_id = session.meeting_id.clone();

    session.signal_stop_and_join();

    let clean = state.settings.read().cancel_my_noise;

    let (mode, processing) = {
        let mut meetings = state.meetings.write();
        let m = meetings
            .get_mut(&meeting_id)
            .ok_or_else(|| AppError::MeetingNotFound(meeting_id.clone()))?;
        m.ended_at = Some(Utc::now());
        m.audio_path = session.audio_path.take();
        m.video_path = session.video_path.take();
        m.timeline.push(TimelineMarker {
            id: format!("t_{}", uuid::Uuid::new_v4().simple()),
            label: "Recording ended".into(),
            at_ms: m.duration_sec * 1000,
            kind: TimelineKind::Leave,
        });
        m.status = MeetingStatus::Processing;
        (m.mode, m.clone())
    };
    let audio_path = processing.audio_path.clone();
    Events::updated(app, &processing);

    // Return the recorder to armed/idle immediately.
    {
        let mut st = state.status.write();
        st.active_meeting_id = None;
        st.elapsed_sec = 0;
        st.mic_level = 0.0;
        st.system_level = 0.0;
        st.message = None;
        st.state = if st.mode == CaptureMode::Off {
            RecorderState::Idle
        } else {
            RecorderState::Armed
        };
        Events::status(app, &st);
    }

    // Post-processing off the command thread.
    let app2 = app.clone();
    let state2 = state.clone();
    let id2 = meeting_id.clone();
    std::thread::Builder::new()
        .name("meetapp-finalize".into())
        .spawn(move || {
            if clean {
                if let Some(path) = &audio_path {
                    if let Err(e) = audio::clean_wav_file(std::path::Path::new(path), true) {
                        tracing::warn!("noise cleaning failed: {e}");
                    }
                }
            }

            if mode == CaptureMode::Transcribe {
                if let Err(e) = crate::core::cloud::run_transcription(&app2, &state2, &id2, true) {
                    tracing::warn!("auto-transcription failed: {e}");
                }
            } else {
                let updated = {
                    let mut meetings = state2.meetings.write();
                    meetings.get_mut(&id2).map(|m| {
                        m.status = MeetingStatus::Ready;
                        m.clone()
                    })
                };
                if let Some(m) = updated {
                    Events::updated(&app2, &m);
                }
            }
        })
        .ok();

    Ok(processing)
}

fn slugify(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "recording".into()
    } else {
        s
    }
}

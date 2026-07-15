use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::recorder;
use crate::error::{AppError, AppResult};
use crate::events::Events;
use crate::models::{
    AudioHealth, CaptureMode, DetectedMeeting, Meeting, MeetingPlatform, MeetingStatus,
    RecorderState, RecorderStatus, Settings, SettingsPatch,
};
use crate::state::AppState;

// ------------------------------------------------------------------ queries

#[tauri::command]
pub fn get_meetings(state: State<'_, AppState>) -> Vec<Meeting> {
    state.meetings_sorted()
}

#[tauri::command]
pub fn get_meeting(state: State<'_, AppState>, id: String) -> Option<Meeting> {
    state.meetings.read().get(&id).cloned()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().clone()
}

#[tauri::command]
pub fn get_recorder_status(state: State<'_, AppState>) -> RecorderStatus {
    state.status.read().clone()
}

#[tauri::command]
pub fn get_detected_meetings(state: State<'_, AppState>) -> Vec<DetectedMeeting> {
    state.detected.read().values().cloned().collect()
}

// --------------------------------------------------------------- settings

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, patch: SettingsPatch) -> Settings {
    let mut s = state.settings.write();
    s.apply(patch);
    s.clone()
}

// ---------------------------------------------------------------- recorder

#[tauri::command]
pub fn set_mode(app: AppHandle, state: State<'_, AppState>, mode: CaptureMode) -> RecorderStatus {
    {
        state.settings.write().default_mode = mode;
    }
    let snapshot = {
        let mut st = state.status.write();
        st.mode = mode;
        match mode {
            CaptureMode::Off => {
                if st.state != RecorderState::Recording {
                    st.state = RecorderState::Idle;
                }
            }
            _ => {
                if st.state == RecorderState::Idle {
                    st.state = RecorderState::Armed;
                }
            }
        }
        st.clone()
    };
    Events::status(&app, &snapshot);
    snapshot
}

#[tauri::command]
pub fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    platform: Option<MeetingPlatform>,
    meeting_id: Option<String>,
) -> AppResult<RecorderStatus> {
    let mode = {
        let st = state.status.read();
        if st.mode == CaptureMode::Off {
            CaptureMode::Record
        } else {
            st.mode
        }
    };
    recorder::start(
        &app,
        state.inner(),
        title.unwrap_or_else(|| "Live recording".into()),
        platform.unwrap_or(MeetingPlatform::Unknown),
        mode,
        meeting_id,
    )?;
    Ok(state.status.read().clone())
}

/// Set the capture volume (input gain). Takes effect immediately — the audio
/// pipeline reads this every buffer — so the volume control works mid-recording.
/// Clamped to `[0, MAX_INPUT_GAIN]`. Returns the updated status.
#[tauri::command]
pub fn set_input_gain(app: AppHandle, state: State<'_, AppState>, gain: f32) -> RecorderStatus {
    let clamped = if gain.is_finite() {
        gain.clamp(0.0, crate::models::MAX_INPUT_GAIN)
    } else {
        1.0
    };
    state
        .recording_gain
        .store(clamped.to_bits(), std::sync::atomic::Ordering::Relaxed);
    let snapshot = {
        let mut st = state.status.write();
        st.input_gain = clamped;
        st.clone()
    };
    Events::status(&app, &snapshot);
    snapshot
}

#[tauri::command]
pub fn stop_capture(app: AppHandle, state: State<'_, AppState>) -> AppResult<Option<Meeting>> {
    match recorder::stop(&app, state.inner()) {
        Ok(m) => Ok(Some(m)),
        Err(AppError::NotRecording) => Ok(None),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub fn capture_detected(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RecorderStatus> {
    let det = state
        .detected
        .read()
        .get(&id)
        .cloned()
        .ok_or_else(|| AppError::Other(format!("detection not found: {id}")))?;
    let mode = {
        let st = state.status.read();
        if st.mode == CaptureMode::Off {
            CaptureMode::Record
        } else {
            st.mode
        }
    };
    recorder::start(&app, state.inner(), det.title, det.platform, mode, Some(id))?;
    Ok(state.status.read().clone())
}

#[tauri::command]
pub fn dismiss_detected(state: State<'_, AppState>, id: String) {
    state.detected.write().remove(&id);
}

#[derive(Serialize)]
pub struct Ack {
    ok: bool,
}

#[tauri::command]
pub fn send_bot(url: Option<String>) -> Ack {
    // A real implementation dispatches a headless meeting bot to join the call.
    tracing::info!("send_bot requested: {url:?}");
    Ack { ok: true }
}

// ----------------------------------------------------------------- meetings

#[tauri::command]
pub fn toggle_meeting_flag(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    flag: String,
) -> AppResult<Meeting> {
    let meeting = {
        let mut meetings = state.meetings.write();
        let m = meetings
            .get_mut(&id)
            .ok_or_else(|| AppError::MeetingNotFound(id.clone()))?;
        match flag.as_str() {
            "locked" => m.is_locked = !m.is_locked,
            "starred" => m.is_starred = !m.is_starred,
            "bookmarked" => m.is_bookmarked = !m.is_bookmarked,
            other => return Err(AppError::Other(format!("unknown flag: {other}"))),
        }
        m.clone()
    };
    Events::updated(&app, &meeting);
    Ok(meeting)
}

#[tauri::command]
pub fn rename_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> AppResult<Meeting> {
    let meeting = {
        let mut meetings = state.meetings.write();
        let m = meetings
            .get_mut(&id)
            .ok_or_else(|| AppError::MeetingNotFound(id.clone()))?;
        m.title = title;
        m.clone()
    };
    Events::updated(&app, &meeting);
    Ok(meeting)
}

#[tauri::command]
pub fn update_action_item(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    item_id: String,
    done: bool,
) -> AppResult<Meeting> {
    let meeting = {
        let mut meetings = state.meetings.write();
        let m = meetings
            .get_mut(&meeting_id)
            .ok_or_else(|| AppError::MeetingNotFound(meeting_id.clone()))?;
        for item in m.action_items.iter_mut() {
            if item.id == item_id {
                item.done = done;
            }
        }
        m.clone()
    };
    Events::updated(&app, &meeting);
    Ok(meeting)
}

#[tauri::command]
pub fn delete_meeting(state: State<'_, AppState>, id: String) {
    state.meetings.write().remove(&id);
}

/// On-demand transcription via AssemblyAI. Runs off the async runtime so the UI
/// stays responsive; the transcript arrives on completion (also via
/// `meeting://updated`).
#[tauri::command]
pub async fn transcribe_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Meeting> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::cloud::run_transcription(&app, &st, &id, false)
    })
    .await
    .map_err(|e| AppError::Other(format!("transcription task failed: {e}")))?
}

/// On-demand summarization via Groq (falls back to the local heuristic when no
/// Groq key is configured).
#[tauri::command]
pub async fn summarize_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Meeting> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::cloud::run_summarization(&app, &st, &id)
    })
    .await
    .map_err(|e| AppError::Other(format!("summarization task failed: {e}")))?
}

/// Error surfaced (in UI and backend) when an audio action is requested for a
/// meeting whose audio isn't available — no file was saved, or it's gone from
/// disk. Phrased so the UI can show it verbatim.
const AUDIO_UNAVAILABLE: &str =
    "Audio isn't available for this meeting, so it can't be enhanced or cleaned yet.";

/// Resolve a meeting's audio file, returning an error if the audio isn't actually
/// available (no path recorded, or the file is missing). This is the single gate
/// both audio-processing commands use so they never run without real audio.
fn require_available_audio(
    state: &AppState,
    id: &str,
) -> AppResult<(String, Meeting)> {
    let (audio_path, meeting) = {
        let meetings = state.meetings.read();
        let m = meetings
            .get(id)
            .ok_or_else(|| AppError::MeetingNotFound(id.to_string()))?;
        (m.audio_path.clone(), m.clone())
    };
    let path = audio_path
        .filter(|p| std::path::Path::new(p).exists())
        .ok_or_else(|| AppError::Other(AUDIO_UNAVAILABLE.into()))?;
    Ok((path, meeting))
}

/// Enhance a saved recording's audio on demand: loudness-normalizes the WAV in
/// place so a quiet recording becomes clearly audible, without clipping/distorting
/// speech (see [`crate::audio::normalize_wav_file`]). Runs off the async runtime
/// since it rewrites the whole file. Only boosts — never makes audio quieter.
/// Refuses (with [`AUDIO_UNAVAILABLE`]) when the meeting has no available audio.
#[tauri::command]
pub async fn enhance_meeting_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Meeting> {
    let (path, meeting) = require_available_audio(state.inner(), &id)?;

    tauri::async_runtime::spawn_blocking(move || {
        crate::audio::normalize_wav_file(std::path::Path::new(&path))
    })
    .await
    .map_err(|e| AppError::Other(format!("enhance task failed: {e}")))??;

    Events::updated(&app, &meeting);
    Ok(meeting)
}

/// Noise-cancel a saved recording on demand: runs AI noise suppression (RNNoise)
/// over the WAV in place (see [`crate::audio::clean_wav_file`]). Runs off the
/// async runtime. Refuses (with [`AUDIO_UNAVAILABLE`]) when audio isn't available.
#[tauri::command]
pub async fn clean_meeting_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Meeting> {
    let (path, meeting) = require_available_audio(state.inner(), &id)?;

    tauri::async_runtime::spawn_blocking(move || {
        crate::audio::clean_wav_file(std::path::Path::new(&path), true)
    })
    .await
    .map_err(|e| AppError::Other(format!("noise-cancellation task failed: {e}")))??;

    Events::updated(&app, &meeting);
    Ok(meeting)
}

// ------------------------------------------------------------- audio health

/// Probe whether the Windows shared-mode audio engine is healthy, so the UI can
/// offer a one-click repair when a broken enhancement is blocking capture.
#[tauri::command]
pub fn audio_health() -> AudioHealth {
    #[cfg(windows)]
    {
        crate::platform::windows::audio_repair::probe()
    }
    #[cfg(not(windows))]
    {
        AudioHealth {
            supported: false,
            detail: "Audio repair is only needed on Windows.".into(),
            ..Default::default()
        }
    }
}

/// Disable the broken audio enhancement on the default endpoints and restart
/// Windows Audio (elevated via UAC). Reversible; installs nothing.
#[tauri::command]
pub fn repair_audio() -> AppResult<String> {
    #[cfg(windows)]
    {
        crate::platform::windows::audio_repair::repair().map_err(AppError::Other)
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Other(
            "Audio repair is only available on Windows.".into(),
        ))
    }
}

/// Open the classic Windows Sound control panel for the manual enhancement toggle.
#[tauri::command]
pub fn open_sound_settings() -> AppResult<()> {
    #[cfg(windows)]
    {
        crate::platform::windows::audio_repair::open_sound_settings().map_err(AppError::Other)
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Other("Only available on Windows.".into()))
    }
}

#[tauri::command]
pub fn open_recordings_folder(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let dir = state.settings.read().save_directory.clone();
    let _ = std::fs::create_dir_all(&dir);
    app.opener()
        .open_path(dir, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;
    Ok(())
}

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
    TimelineMarker, TranscriptSegment,
};
use crate::state::AppState;

/// Control handle for the active capture, stored in [`crate::state::Inner`].
pub struct RecordingSession {
    pub meeting_id: String,
    stop: Arc<AtomicBool>,
    loop_join: Option<JoinHandle<()>>,
    pub audio_path: Option<String>,
    pub video_path: Option<String>,
    /// Detection key this recording was auto-started for, if any. The detection
    /// loop uses it to auto-stop the recording when that meeting ends. `None` for
    /// manual "Record Live" sessions, which run until the user stops them.
    pub from_detection: Option<String>,
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

    // Every recording starts with the mic live; the user opts into muting during
    // the session. Clears any manual mute left over from a previous recording.
    state
        .manual_mic_mute
        .store(false, Ordering::Relaxed);

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
        match Recorder::start(&path, capture_system_audio, state.recording_gain.clone()) {
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
    // Same for video: screen capture is an opt-in feature that yields nothing in
    // the default build, so a RecordVideo meeting with no video file must report
    // `has_video = false`. Otherwise the detail page shows an empty video surface
    // (and suppresses the audio player), making the recording look unplayable.
    meeting.has_video = mode.saves_video() && meeting.video_path.is_some();
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
        st.mic_muted = false;
        st.input_gain = f32::from_bits(state.recording_gain.load(Ordering::Relaxed));
        // Not live yet — the capture loop flips this on once audio actually flows.
        // If no device opened at all, there's nothing to wait for, so report ready.
        st.audio_ready = audio_unavailable;
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
    if let Some(id) = &from_detection {
        if let Some(det) = state.detected.write().get_mut(id) {
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
        from_detection,
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
            // Tick faster than once a second so the "starting… → live" transition
            // and the level meters feel responsive; elapsed seconds are derived
            // from the tick count (4 ticks = 1 s).
            const TICK: Duration = Duration::from_millis(250);
            const TICKS_PER_SEC: u64 = 1000 / 250;
            let mut ticks: u64 = 0;
            let mut elapsed: u64 = 0;
            let mut was_ready = false;

            // System capture only reflects the output-device mute state on the
            // endpoint-loopback fallback; process loopback is immune, so we only
            // watch mute there (and avoid false "you're muted" warnings otherwise).
            let watch_mute = capture
                .as_ref()
                .map(|c| c.system_on_endpoint_fallback())
                .unwrap_or(false);
            // The status message set at start (e.g. the exclusive-mic warning); the
            // mute warning overrides it while muted, then restores it.
            let base_message = state.status.read().message.clone();
            let mut system_muted = false;
            // Whether the user currently has the microphone muted. Polled each
            // second and pushed to the pipeline, which then records the mic as
            // silence (system audio keeps recording).
            let mut mic_muted = false;

            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                ticks += 1;
                let prev_elapsed = elapsed;
                elapsed = ticks / TICKS_PER_SEC;

                // Re-check output-device mute about once a second (cheap COM call).
                if watch_mute && ticks % TICKS_PER_SEC == 0 {
                    let now_muted = system_output_muted();
                    if now_muted != system_muted {
                        system_muted = now_muted;
                        tracing::info!("system audio: output-device muted = {now_muted}");
                    }
                }

                // Re-check the microphone mute every tick (250 ms) and push it to
                // the pipeline, so muting/unmuting takes effect almost immediately.
                // The mic counts as muted when EITHER the OS/device is muted OR the
                // user muted it in-app (`set_mic_mute`) — the latter is what makes a
                // Teams/Zoom in-app mute actually stop the recording, since those
                // never touch the Windows endpoint. While muted, the pipeline records
                // the mic as silence and opens no "You" segment; system audio keeps
                // recording. Runs regardless of the system-mute watch above.
                {
                    let now_mic_muted = microphone_muted()
                        || state.manual_mic_mute.load(Ordering::Relaxed);
                    if let Some(c) = capture.as_ref() {
                        c.set_mic_muted(now_mic_muted);
                    }
                    if now_mic_muted != mic_muted {
                        mic_muted = now_mic_muted;
                        tracing::info!("microphone: user mute = {mic_muted}");
                    }
                }

                // Real input levels from the active capture (0 when none opened),
                // and whether capture has actually gone live yet.
                let mic_level = capture.as_ref().map(|c| c.mic_level()).unwrap_or(0.0);
                let system_level = capture.as_ref().map(|c| c.system_level()).unwrap_or(0.0);
                let audio_ready = capture.as_ref().map(|c| c.audio_ready()).unwrap_or(true);
                if audio_ready && !was_ready {
                    was_ready = true;
                    tracing::info!("recorder: audio is now live");
                }

                if elapsed != prev_elapsed {
                    if let Some(m) = state.meetings.write().get_mut(&meeting_id) {
                        m.duration_sec = elapsed;
                    }
                }
                {
                    let mut st = state.status.write();
                    st.elapsed_sec = elapsed;
                    // While muted, force the mic meter to 0 immediately (the pipeline
                    // also zeroes it, but polling lags by up to a second).
                    st.mic_level = if mic_muted { 0.0 } else { mic_level };
                    st.system_level = system_level;
                    st.mic_muted = mic_muted;
                    st.audio_ready = st.audio_ready || audio_ready;
                    if watch_mute {
                        st.message = if system_muted {
                            Some(
                                "Your speakers are muted, so the other participants' audio isn't \
                                 being recorded on this PC — unmute to capture system sound."
                                    .into(),
                            )
                        } else {
                            base_message.clone()
                        };
                    }
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

/// Offline transcript fallback for Transcribe mode when no cloud STT is
/// configured: fills in the built-in simulated transcript, derives participants,
/// drops the throwaway temp audio (Transcribe retains none), and marks the meeting
/// Ready — so the mode completes end-to-end without a network call or API key
/// instead of hanging in "Processing".
fn finalize_offline_transcript(app: &AppHandle, state: &AppState, meeting_id: &str) {
    let segments = crate::core::transcription::simulated_segments();
    let updated = {
        let mut meetings = state.meetings.write();
        match meetings.get_mut(meeting_id) {
            Some(m) => {
                let participants = participants_from_transcript(&segments);
                m.transcript = segments;
                if !participants.is_empty() {
                    m.participants = participants;
                }
                if let Some(path) = m.audio_path.take() {
                    let _ = std::fs::remove_file(&path);
                }
                m.has_audio = false;
                m.status = MeetingStatus::Ready;
                Some(m.clone())
            }
            None => None,
        }
    };
    if let Some(m) = updated {
        Events::updated(app, &m);
    }
}

/// Build a talk-time participant list from a finished transcript (one entry per
/// speaker, in first-appearance order).
fn participants_from_transcript(segments: &[TranscriptSegment]) -> Vec<Participant> {
    use std::collections::HashMap;

    let mut order: Vec<String> = Vec::new();
    let mut talk: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    for s in segments {
        let dur = s.end_ms.saturating_sub(s.start_ms);
        if !talk.contains_key(&s.speaker) {
            order.push(s.speaker.clone());
        }
        *talk.entry(s.speaker.clone()).or_insert(0) += dur;
        total += dur;
    }
    order
        .into_iter()
        .map(|name| {
            let ms = talk[&name];
            Participant {
                id: format!("p_{}", uuid::Uuid::new_v4().simple()),
                name,
                talk_ratio: (total > 0).then(|| ms as f32 / total as f32),
            }
        })
        .collect()
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
    let video_path = processing.video_path.clone();
    Events::updated(app, &processing);

    // Return the recorder to armed/idle immediately.
    {
        let mut st = state.status.write();
        st.active_meeting_id = None;
        st.elapsed_sec = 0;
        st.mic_level = 0.0;
        st.system_level = 0.0;
        st.mic_muted = false;
        st.audio_ready = false;
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
            // ── SINGLE noise-suppression decision point ──────────────────────
            // DeepFilterNet is the primary engine; RNNoise is the fallback used
            // only when DeepFilterNet is unavailable/fails/produces invalid output.
            // Never both (see `audio::suppress_noise`). This runs for EVERY mode
            // that produced audio — including Transcribe's temp file — so the very
            // same processed audio is what we save for playback and/or send to
            // transcription. `run_transcription` no longer denoises: this is the
            // only suppression pass in the pipeline.
            if let Some(path) = &audio_path {
                // Toggles: "cancel my noise" → mic side, "cancel others' noise" →
                // far-end side. Read here so a mid-session change still applies.
                let (cancel_system, cancel_mic) = {
                    let s = state2.settings.read();
                    (s.cancel_others_noise, s.cancel_my_noise)
                };
                audio::suppress_noise(std::path::Path::new(path), cancel_system, cancel_mic);
            }

            // Loudness/dead-air correction for PLAYBACK — this is AGC + trimming,
            // not noise suppression, so it stays scoped to modes that KEEP audio.
            // Transcribe mode's throwaway temp doesn't need it (and there's nothing
            // to play back).
            if mode.saves_audio() {
                if let Some(path) = &audio_path {
                    let p = std::path::Path::new(path);
                    // Trim leading/trailing dead air before enhancing, so the AGC
                    // boost is measured over real content. Skipped for video: the
                    // A/V mux uses ffmpeg `-shortest`, so a shortened audio track
                    // would truncate the video.
                    if !mode.saves_video() {
                        if let Err(e) = audio::trim_silence_wav_file(p) {
                            tracing::warn!("silence trim failed: {e}");
                        }
                    }
                    if let Err(e) = audio::normalize_wav_file(p) {
                        tracing::warn!("audio enhancement failed: {e}");
                    }
                }
            }

            // Record Video: the screen capture is a video-only MP4 (the capture
            // API doesn't record sound). Now that the mic+system WAV is finalized,
            // mux it into the video so the saved recording has audio. If ffmpeg
            // isn't available the mux is skipped and both files are kept as-is.
            if mode.saves_video() {
                if let (Some(vpath), Some(apath)) = (video_path.as_ref(), audio_path.as_ref()) {
                    if let Some(combined) = audio::mux_audio_into_video(vpath, apath) {
                        // Replace the silent video-only file with the combined one.
                        let _ = std::fs::remove_file(vpath);
                        if let Some(m) = state2.meetings.write().get_mut(&id2) {
                            m.video_path = Some(combined);
                        }
                    }
                }
            }

            if mode == CaptureMode::Transcribe {
                if let Err(e) = crate::core::cloud::run_transcription(&app2, &state2, &id2, true) {
                    tracing::warn!("auto-transcription failed: {e}");
                    // Never leave the meeting stuck in "Processing". If a cloud key
                    // is configured, run_transcription already marked it Failed (a
                    // genuine error worth surfacing). If none is — the offline
                    // default — fall back to the built-in transcript so Transcribe
                    // mode still completes end-to-end, matching the browser mock and
                    // the documented offline behavior.
                    let has_cloud_key = {
                        let s = state2.settings.read();
                        crate::core::cloud::resolve_key(
                            &s.assemblyai_api_key,
                            "ASSEMBLYAI_API_KEY",
                        )
                        .is_some()
                    };
                    if !has_cloud_key {
                        finalize_offline_transcript(&app2, &state2, &id2);
                    }
                }
                // Transcribe mode NEVER retains audio. On success `run_transcription`
                // already deleted the temp WAV, and the no-key fallback deletes it
                // too — but a cloud failure *with* a key configured leaves it behind.
                // Delete it unconditionally here so the audio is never saved, and
                // clear any dangling path/flag on the record.
                if let Some(path) = &audio_path {
                    let _ = std::fs::remove_file(path);
                }
                if let Some(m) = state2.meetings.write().get_mut(&id2) {
                    m.audio_path = None;
                    m.has_audio = false;
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
                    // Persist the finished recording's metadata so it survives a
                    // restart and shows up in the library next launch.
                    state2.persist_meeting(&m);
                    Events::updated(&app2, &m);
                }
            }
        })
        .ok();

    Ok(processing)
}

/// Whether the default output device is muted right now. Windows-only; always
/// `false` elsewhere (and only ever consulted on the Windows endpoint-loopback
/// fallback). Read-only — never changes the system mute state.
#[cfg(windows)]
fn system_output_muted() -> bool {
    crate::platform::windows::system_audio::default_render_muted().unwrap_or(false)
}
#[cfg(not(windows))]
fn system_output_muted() -> bool {
    false
}

/// Whether the microphone (default capture endpoint) is muted at the OS/device
/// level right now. Windows-only; always `false` elsewhere. When it can't be
/// determined we treat the mic as *not* muted, so an inability to read the mute
/// state never silently drops the microphone. Read-only.
#[cfg(windows)]
fn microphone_muted() -> bool {
    crate::platform::windows::system_audio::default_capture_muted().unwrap_or(false)
}
#[cfg(not(windows))]
fn microphone_muted() -> bool {
    false
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

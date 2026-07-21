use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use tauri::AppHandle;

use crate::core::recorder;
use crate::events::Events;
use crate::models::{CaptureMode, DetectedMeeting, MeetingPlatform, RecorderState};
use crate::platform;
use crate::state::AppState;

/// How often to poll for in-progress meetings.
const POLL_INTERVAL: Duration = Duration::from_secs(4);

/// Consecutive absent polls before a detected meeting is treated as really ended
/// and its auto-recording is stopped. Debounces transient detection drops (e.g.
/// briefly switching browser tabs away from a Google Meet call) so a recording
/// isn't cut short. At `POLL_INTERVAL` (4s) this is ~8s.
const END_GRACE_TICKS: u32 = 2;

fn key(platform: MeetingPlatform) -> String {
    format!("det_{platform:?}")
}

/// Start the background detection loop. Detached for the app's lifetime.
pub fn spawn(app: AppHandle, state: AppState) {
    std::thread::Builder::new()
        .name("meetapp-detect".into())
        .spawn(move || {
            // Consecutive polls the active auto-recording's source meeting has been
            // absent; drives the debounced auto-stop (see `END_GRACE_TICKS`).
            let mut missing_ticks: u32 = 0;
            loop {
                std::thread::sleep(POLL_INTERVAL);
                tick(&app, &state, &mut missing_ticks);
            }
        })
        .expect("failed to spawn detection loop");
}

fn tick(app: &AppHandle, state: &AppState, missing_ticks: &mut u32) {
    let mode = state.status.read().mode;

    // When the note taker is off, surface no detections.
    if mode == CaptureMode::Off {
        *missing_ticks = 0;
        clear_all(app, state);
        return;
    }

    let candidates = platform::detect::scan();
    let current: HashSet<String> = candidates.iter().map(|c| key(c.platform)).collect();

    // New detections.
    for c in &candidates {
        let k = key(c.platform);
        if state.detected.read().contains_key(&k) {
            continue;
        }
        let det = DetectedMeeting {
            id: k.clone(),
            platform: c.platform,
            title: c.title.clone(),
            process_name: c.process_name.clone(),
            detected_at: Utc::now(),
            capturing: false,
        };
        state.detected.write().insert(k.clone(), det.clone());

        {
            let mut st = state.status.write();
            if st.state == RecorderState::Armed {
                st.state = RecorderState::Detecting;
                Events::status(app, &st);
            }
        }
        Events::detected(app, &det);

        // Auto-record if enabled and nothing else is capturing.
        let auto = state.settings.read().auto_record_detected;
        if auto && state.session.lock().is_none() {
            if let Err(e) = recorder::start(
                app,
                state,
                det.title.clone(),
                det.platform,
                mode,
                Some(k.clone()),
            ) {
                tracing::warn!("auto-record failed: {e}");
            }
        }
    }

    // Ended detections (windows that disappeared).
    let removed: Vec<String> = {
        let det = state.detected.read();
        det.keys()
            .filter(|k| !current.contains(*k))
            .cloned()
            .collect()
    };
    for k in removed {
        state.detected.write().remove(&k);
        Events::ended(app, k);
    }

    // Auto-stop the recording once its source meeting has ended.
    maybe_auto_stop(app, state, &current, missing_ticks);
}

/// Stop the recording that was auto-started for a detected meeting once that
/// meeting's window has been gone for [`END_GRACE_TICKS`] consecutive polls.
///
/// Only affects recordings tied to a detection (`RecordingSession::from_detection`);
/// a manual "Record Live" session has no source detection and is left running
/// until the user stops it. The grace period debounces flaky detection so a brief
/// drop (e.g. a tab switch) doesn't end the recording prematurely.
fn maybe_auto_stop(
    app: &AppHandle,
    state: &AppState,
    current: &HashSet<String>,
    missing: &mut u32,
) {
    // Read (and release) the session lock before calling `recorder::stop`, which
    // re-locks it.
    let from_detection = state
        .session
        .lock()
        .as_ref()
        .and_then(|s| s.from_detection.clone());

    let Some(key) = from_detection else {
        *missing = 0;
        return;
    };
    if current.contains(&key) {
        *missing = 0; // meeting still present — reset the grace counter
        return;
    }

    *missing += 1;
    if *missing < END_GRACE_TICKS {
        return;
    }
    *missing = 0;
    tracing::info!("detected meeting ended — auto-stopping the recording");
    match recorder::stop(app, state) {
        Ok(m) => tracing::info!("auto-stopped recording for meeting {}", m.id),
        Err(e) => tracing::warn!("auto-stop failed: {e}"),
    }
}

fn clear_all(app: &AppHandle, state: &AppState) {
    let ids: Vec<String> = {
        let det = state.detected.read();
        if det.is_empty() {
            return;
        }
        det.keys().cloned().collect()
    };
    state.detected.write().clear();
    for id in ids {
        Events::ended(app, id);
    }
}

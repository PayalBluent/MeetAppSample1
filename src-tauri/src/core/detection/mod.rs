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

fn key(platform: MeetingPlatform) -> String {
    format!("det_{platform:?}")
}

/// Start the background detection loop. Detached for the app's lifetime.
pub fn spawn(app: AppHandle, state: AppState) {
    std::thread::Builder::new()
        .name("meetapp-detect".into())
        .spawn(move || loop {
            std::thread::sleep(POLL_INTERVAL);
            tick(&app, &state);
        })
        .expect("failed to spawn detection loop");
}

fn tick(app: &AppHandle, state: &AppState) {
    let mode = state.status.read().mode;

    // When the note taker is off, surface no detections.
    if mode == CaptureMode::Off {
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

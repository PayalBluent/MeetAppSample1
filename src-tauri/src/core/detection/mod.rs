use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use tauri::AppHandle;

use crate::core::recorder;
use crate::events::Events;
use crate::models::{CaptureMode, DetectedMeeting, MeetingPlatform, RecorderState};
use crate::platform;
use crate::state::AppState;

/// How often to poll for in-progress meetings. Kept short so a meeting is picked
/// up (and auto-recording starts) within a couple of seconds of it opening.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Consecutive absent polls before a detected meeting is treated as really ended
/// and its auto-recording is stopped. Absence here is measured with the *lenient*
/// active-platform check ([`platform::detect::active_platforms`]), which stays true
/// for the whole call (it recognises an in-call window even after its title drops
/// the "meeting"/"call" token) and only goes false once the app returns to an idle
/// state. Because that signal reliably tracks the meeting, this grace only has to
/// debounce the brief moment the meeting window closes — not bridge long in-call
/// title gaps — so it can be short. At `POLL_INTERVAL` (2s) this is ~8s: the
/// recording stops within roughly 8-10s of the meeting actually ending, while two
/// extra polls of margin keep a momentary flicker from ever ending it early.
const END_GRACE_TICKS: u32 = 4;

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
    let (mode, auto_record) = {
        let st = state.status.read();
        let s = state.settings.read();
        (st.mode, s.auto_record_detected)
    };

    // Detection is active when the note taker is armed (any non-Off mode) OR when
    // "Auto-record detected meetings" is enabled. This is the key: a user who left
    // the mode on Off but turned auto-record on still gets meetings detected and
    // captured — previously the Off gate blocked detection before auto-record was
    // ever considered, so the toggle did nothing.
    if mode == CaptureMode::Off && !auto_record {
        *missing_ticks = 0;
        clear_all(app, state);
        // Fully off → the pill should read "Idle" (unless a manual capture is still
        // finishing up).
        set_baseline_state(app, state, RecorderState::Idle);
        return;
    }

    // The mode a detected meeting is captured with. When the note taker is Off but
    // auto-record is on, fall back to Record (the standard capturing mode, and the
    // only one currently available in the UI) — mirroring how "Record Live" forces
    // Record when the mode is Off.
    let capture_mode = if mode == CaptureMode::Off {
        CaptureMode::Record
    } else {
        mode
    };

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
            // Reflect detection in the UI whether the recorder was armed (non-Off
            // mode) or idle (Off mode with auto-record on).
            if st.state == RecorderState::Armed || st.state == RecorderState::Idle {
                st.state = RecorderState::Detecting;
                Events::status(app, &st);
            }
        }
        Events::detected(app, &det);

        // Auto-record if enabled and nothing else is capturing.
        if auto_record && state.session.lock().is_none() {
            if let Err(e) = recorder::start(
                app,
                state,
                det.title.clone(),
                det.platform,
                capture_mode,
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

    // Auto-stop the recording once its source meeting has ended. This uses the
    // *lenient* active-platform set, not the strict `current` above: during a call
    // a Teams window title often drops the "meeting"/"call" token, and the strict
    // set would then think the meeting ended and cut the recording off mid-call.
    // The lenient set keeps an in-call window alive and only drops once the app is
    // back on an idle nav section — so the recording spans the whole meeting and
    // stops shortly after it truly ends. New detections above stay on `current`
    // (strict), so nothing is ever *started* falsely.
    let active_keys: HashSet<String> = platform::detect::active_platforms()
        .into_iter()
        .map(key)
        .collect();
    maybe_auto_stop(app, state, &active_keys, missing_ticks);

    // Keep the status pill honest: while the note taker is active (armed mode or
    // auto-record on) and nothing is currently being captured, show "Armed" (it's
    // actively watching for meetings) rather than the misleading "Idle". A live
    // detection shows "Meeting detected"; capture/processing states are left alone.
    let baseline = if state.detected.read().is_empty() {
        RecorderState::Armed
    } else {
        RecorderState::Detecting
    };
    set_baseline_state(app, state, baseline);
}

/// Move the recorder to a non-capturing baseline state (`Idle`, `Armed`, or
/// `Detecting`) without disturbing an active capture. Recording/Processing/Error
/// are owned by the recorder itself, so they're never overridden here; nor is a
/// state that already matches. Emits a status event only when something changed.
fn set_baseline_state(app: &AppHandle, state: &AppState, desired: RecorderState) {
    // Never touch the state while a capture session is live — the recorder owns
    // Recording/Processing transitions.
    if state.session.lock().is_some() {
        return;
    }
    let mut st = state.status.write();
    match st.state {
        RecorderState::Recording | RecorderState::Processing | RecorderState::Error => return,
        _ if st.state == desired => return,
        _ => {
            st.state = desired;
            Events::status(app, &st);
        }
    }
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

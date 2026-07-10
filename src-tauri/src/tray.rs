use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::core::recorder;
use crate::events::Events;
use crate::models::{CaptureMode, RecorderState};
use crate::state::AppState;

/// Build the system-tray icon, menu, and event handlers.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open MeetApp", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "Show quick panel", true, None::<&str>)?;

    let m_transcribe =
        MenuItem::with_id(app, "mode_transcribe", "Transcribe", true, None::<&str>)?;
    let m_record = MenuItem::with_id(app, "mode_record", "Record", true, None::<&str>)?;
    let m_video = MenuItem::with_id(app, "mode_recordVideo", "Record Video", true, None::<&str>)?;
    let m_off = MenuItem::with_id(app, "mode_off", "Off", true, None::<&str>)?;
    let mode_menu = Submenu::with_items(
        app,
        "Mode",
        true,
        &[&m_transcribe, &m_record, &m_video, &m_off],
    )?;

    let toggle = MenuItem::with_id(app, "toggle_record", "Start recording", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit MeetApp", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open,
            &panel,
            &PredefinedMenuItem::separator(app)?,
            &mode_menu,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("MeetApp — idle")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_panel(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Reflect the current recorder state in the tray tooltip + toggle label.
pub fn refresh(app: &AppHandle) {
    let state = app.state::<AppState>();
    let status = state.status.read().clone();
    let tip = match status.state {
        RecorderState::Recording => "MeetApp — recording",
        RecorderState::Processing => "MeetApp — processing",
        RecorderState::Detecting => "MeetApp — meeting detected",
        RecorderState::Armed => "MeetApp — armed",
        RecorderState::Error => "MeetApp — error",
        RecorderState::Idle => "MeetApp — idle",
    };
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main(app),
        "panel" => toggle_panel(app),
        "quit" => app.exit(0),
        "toggle_record" => toggle_record(app),
        "mode_transcribe" => set_mode(app, CaptureMode::Transcribe),
        "mode_record" => set_mode(app, CaptureMode::Record),
        "mode_recordVideo" => set_mode(app, CaptureMode::RecordVideo),
        "mode_off" => set_mode(app, CaptureMode::Off),
        _ => {}
    }
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn toggle_panel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("panel") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

fn set_mode(app: &AppHandle, mode: CaptureMode) {
    let state = app.state::<AppState>();
    state.settings.write().default_mode = mode;
    let snapshot = {
        let mut st = state.status.write();
        st.mode = mode;
        match mode {
            CaptureMode::Off if st.state != RecorderState::Recording => {
                st.state = RecorderState::Idle
            }
            CaptureMode::Off => {}
            _ if st.state == RecorderState::Idle => st.state = RecorderState::Armed,
            _ => {}
        }
        st.clone()
    };
    Events::status(app, &snapshot);
    refresh(app);
}

fn toggle_record(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recording = state.session.lock().is_some();
    if recording {
        let _ = recorder::stop(app, state.inner());
    } else {
        let mode = {
            let st = state.status.read();
            if st.mode == CaptureMode::Off {
                CaptureMode::Record
            } else {
                st.mode
            }
        };
        let _ = recorder::start(
            app,
            state.inner(),
            "Live recording".into(),
            crate::models::MeetingPlatform::Unknown,
            mode,
            None,
        );
    }
    refresh(app);
}

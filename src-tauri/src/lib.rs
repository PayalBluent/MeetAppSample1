mod audio;
mod commands;
mod core;
mod dotenv;
mod error;
mod events;
mod models;
mod platform;
mod state;
mod tray;

use tauri::{Manager, WindowEvent};

use state::{AppState, Inner};

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    // Load API keys (and any other config) from a `.env` file into the process
    // environment before state is built, so Settings can pick them up.
    dotenv::load();

    let mut builder = tauri::Builder::default();

    // Single-instance must be registered first so a second launch focuses the
    // existing window instead of spawning a duplicate.
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }))
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Recordings live under ~/MeetApp/recordings by default.
            let save_dir = app
                .path()
                .home_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("MeetApp")
                .join("recordings");
            let _ = std::fs::create_dir_all(&save_dir);

            let state: AppState = Inner::new(save_dir);
            app.manage(state.clone());

            // System tray + background detection.
            tray::build(&handle)?;
            core::detection::spawn(handle.clone(), state.clone());

            // Honor "start minimized to tray".
            if state.settings.read().start_minimized {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            tray::refresh(&handle);

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing a window hides it to the tray instead of quitting; the app
            // exits only via the tray's "Quit" item.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_meetings,
            commands::get_meeting,
            commands::get_settings,
            commands::update_settings,
            commands::get_recorder_status,
            commands::get_detected_meetings,
            commands::set_mode,
            commands::start_capture,
            commands::stop_capture,
            commands::capture_detected,
            commands::dismiss_detected,
            commands::send_bot,
            commands::toggle_meeting_flag,
            commands::rename_meeting,
            commands::update_action_item,
            commands::delete_meeting,
            commands::transcribe_meeting,
            commands::summarize_meeting,
            commands::open_recordings_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MeetApp");
}

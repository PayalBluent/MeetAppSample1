mod audio;
mod commands;
mod core;
mod dotenv;
mod error;
mod events;
mod models;
mod platform;
mod recorder;
mod state;
mod tray;

use tauri::{Manager, WindowEvent};

use state::{AppState, Inner};

/// A shared, append-only log file usable as a `tracing` writer from every thread.
struct SharedLogFile(std::sync::Arc<std::sync::Mutex<std::fs::File>>);

impl std::io::Write for SharedLogFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.lock() {
            Ok(mut f) => f.write(buf),
            Err(_) => Ok(buf.len()),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.lock() {
            Ok(mut f) => f.flush(),
            Err(_) => Ok(()),
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Mirror logs to ~/MeetApp/logs/meetapp.log so capture diagnostics — in
    // particular the pipeline's per-source "recording finalized: mic[...],
    // system[...]" verdict — are readable even from a packaged build with no
    // attached console. Falls back to stdout-only if the file can't be opened.
    let log_file = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("MeetApp")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_file);
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file.join("meetapp.log"))
        .ok()
        .map(|f| std::sync::Arc::new(std::sync::Mutex::new(f)));

    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match opened {
        Some(file) => {
            let make_file = move || SharedLogFile(file.clone());
            let _ = builder
                .with_ansi(false)
                .with_writer(std::io::stdout.and(make_file))
                .try_init();
        }
        None => {
            let _ = builder.try_init();
        }
    }
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
        .manage(recorder::RecorderState::default())
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

            let state: AppState = Inner::new(save_dir.clone());
            app.manage(state.clone());

            // Load previously-saved recordings (and import any media already in the
            // folder that has no metadata yet) so the user's past recordings show up
            // and remain accessible across restarts. Existing recordings are never
            // modified — only their sidecar metadata is read or, for orphans, added.
            state.load_persisted_recordings(&save_dir);

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
            commands::set_input_gain,
            commands::set_mic_mute,
            commands::enhance_meeting_audio,
            commands::clean_meeting_audio,
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
            commands::audio_health,
            commands::repair_audio,
            commands::open_sound_settings,
            recorder::start_recording,
            recorder::stop_recording,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MeetApp");
}

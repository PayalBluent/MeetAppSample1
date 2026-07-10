use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::meeting::CaptureMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub default_mode: CaptureMode,
    pub auto_record_detected: bool,
    /// Capture the computer's audio output (speaker) alongside the mic, so the
    /// other call participants are recorded. Windows only; ignored elsewhere.
    pub capture_system_audio: bool,
    pub cancel_my_noise: bool,
    pub cancel_others_noise: bool,
    pub launch_on_startup: bool,
    pub start_minimized: bool,
    pub save_directory: String,
    pub theme: ThemePreference,

    /// Cloud AI credentials (also read from ASSEMBLYAI_API_KEY / GROQ_API_KEY
    /// env vars when blank). Stored in memory only.
    pub assemblyai_api_key: String,
    pub groq_api_key: String,
    pub groq_model: String,
}

impl Settings {
    /// Sensible defaults, saving to `~/MeetApp/recordings`. Cloud credentials are
    /// seeded from the environment (populated from `.env` at startup), so keys can
    /// be configured in one place without retyping them in the UI each launch.
    pub fn with_default_dir(dir: PathBuf) -> Self {
        Settings {
            default_mode: CaptureMode::Off,
            auto_record_detected: true,
            capture_system_audio: true,
            cancel_my_noise: false,
            cancel_others_noise: false,
            launch_on_startup: false,
            start_minimized: false,
            save_directory: dir.to_string_lossy().replace('\\', "/"),
            theme: ThemePreference::Light,
            assemblyai_api_key: env_or_default("ASSEMBLYAI_API_KEY", ""),
            groq_api_key: env_or_default("GROQ_API_KEY", ""),
            groq_model: env_or_default("GROQ_MODEL", "llama-3.3-70b-versatile"),
        }
    }
}

/// Read a trimmed, non-empty environment variable, or fall back to `default`.
fn env_or_default(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Partial patch coming from the frontend `update_settings` command. Every field
/// is optional so callers can update one toggle at a time.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub default_mode: Option<CaptureMode>,
    pub auto_record_detected: Option<bool>,
    pub capture_system_audio: Option<bool>,
    pub cancel_my_noise: Option<bool>,
    pub cancel_others_noise: Option<bool>,
    pub launch_on_startup: Option<bool>,
    pub start_minimized: Option<bool>,
    pub save_directory: Option<String>,
    pub theme: Option<ThemePreference>,
    pub assemblyai_api_key: Option<String>,
    pub groq_api_key: Option<String>,
    pub groq_model: Option<String>,
}

impl Settings {
    pub fn apply(&mut self, patch: SettingsPatch) {
        if let Some(v) = patch.default_mode {
            self.default_mode = v;
        }
        if let Some(v) = patch.auto_record_detected {
            self.auto_record_detected = v;
        }
        if let Some(v) = patch.capture_system_audio {
            self.capture_system_audio = v;
        }
        if let Some(v) = patch.cancel_my_noise {
            self.cancel_my_noise = v;
        }
        if let Some(v) = patch.cancel_others_noise {
            self.cancel_others_noise = v;
        }
        if let Some(v) = patch.launch_on_startup {
            self.launch_on_startup = v;
        }
        if let Some(v) = patch.start_minimized {
            self.start_minimized = v;
        }
        if let Some(v) = patch.save_directory {
            self.save_directory = v;
        }
        if let Some(v) = patch.theme {
            self.theme = v;
        }
        if let Some(v) = patch.assemblyai_api_key {
            self.assemblyai_api_key = v;
        }
        if let Some(v) = patch.groq_api_key {
            self.groq_api_key = v;
        }
        if let Some(v) = patch.groq_model {
            self.groq_model = v;
        }
    }
}

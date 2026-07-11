pub mod detect;
pub mod window_titles;

// System-audio capture via WASAPI. Reached through `audio::capture`.
pub mod system_audio;

// Detect/repair an impaired shared-mode audio engine. Reached through commands.
pub mod audio_repair;

#[cfg(feature = "screen-capture")]
pub mod screen;

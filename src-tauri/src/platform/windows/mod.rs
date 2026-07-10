pub mod detect;
pub mod window_titles;

// System-audio capture via WASAPI. Reached through `audio::capture`.
pub mod system_audio;

#[cfg(feature = "screen-capture")]
pub mod screen;

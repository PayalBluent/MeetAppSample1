//! Platform-isolation layer.
//!
//! The rest of the app depends only on the platform-independent facades in
//! [`detect`] and [`screen`]. Concrete OS implementations live in `windows/`
//! (and, later, `macos/`), so adding macOS support means implementing these two
//! facades — nothing in `core/`, `commands/`, or the UI has to change.

pub mod detect;
pub mod screen;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

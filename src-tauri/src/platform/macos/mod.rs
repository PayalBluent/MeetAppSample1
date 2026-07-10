//! macOS platform stubs.
//!
//! Placeholder for the second-release macOS target. Detection will use
//! `CGWindowListCopyWindowInfo` (window owner + title) and system-audio/screen
//! capture will use ScreenCaptureKit. The facades in `platform::detect` /
//! `platform::screen` already route here under `cfg(target_os = "macos")`, so
//! only these functions need real implementations.

use crate::platform::detect::Candidate;

pub fn detect_scan() -> Vec<Candidate> {
    // TODO(macos): enumerate windows via CGWindowListCopyWindowInfo and classify.
    Vec::new()
}

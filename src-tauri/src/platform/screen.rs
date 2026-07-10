//! Screen + video recording facade.
//!
//! Returns an opaque [`ScreenHandle`] whose `stop()` finalizes the recording.
//! The real Windows backend (Windows.Graphics.Capture via the `windows-capture`
//! crate) is compiled only with the `screen-capture` feature.

/// Opaque control handle for an in-progress screen recording.
pub struct ScreenHandle {
    stopper: Option<Box<dyn FnOnce() + Send>>,
}

impl ScreenHandle {
    pub fn new(stopper: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            stopper: Some(stopper),
        }
    }

    pub fn stop(mut self) {
        if let Some(f) = self.stopper.take() {
            f();
        }
    }
}

/// Begin recording the primary display to `<dir>/<slug>.mp4`.
/// Returns `(handle, output_path)` on success, or `None` when video capture is
/// unavailable (feature disabled, unsupported platform, or no display).
pub fn start(dir: &str, slug: &str) -> Option<(ScreenHandle, String)> {
    #[cfg(all(windows, feature = "screen-capture"))]
    {
        return crate::platform::windows::screen::start(dir, slug);
    }
    #[cfg(not(all(windows, feature = "screen-capture")))]
    {
        let _ = (dir, slug);
        None
    }
}

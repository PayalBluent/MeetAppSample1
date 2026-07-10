//! Windows screen recording via Windows.Graphics.Capture (`windows-capture`).
//!
//! Experimental, opt-in (`--features screen-capture`). Targets `windows-capture`
//! 1.x. If the crate's `Settings::new` arity changes between minor versions,
//! adjust the call in [`start`]; nothing else in the app depends on this file.

use std::time::Instant;

use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    },
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
    },
};

use crate::platform::screen::ScreenHandle;

type CaptureError = Box<dyn std::error::Error + Send + Sync>;

struct Capture {
    encoder: Option<VideoEncoder>,
    #[allow(dead_code)]
    started: Instant,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = String; // output path
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let monitor = Monitor::primary()?;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(monitor.width()?, monitor.height()?),
            AudioSettingsBuilder::default().disabled(false),
            ContainerSettingsBuilder::default(),
            &ctx.flags,
        )?;
        Ok(Self {
            encoder: Some(encoder),
            started: Instant::now(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if let Some(enc) = self.encoder.as_mut() {
            enc.send_frame(frame)?;
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if let Some(enc) = self.encoder.take() {
            enc.finish()?;
        }
        Ok(())
    }
}

pub fn start(dir: &str, slug: &str) -> Option<(ScreenHandle, String)> {
    let path = format!("{dir}/{slug}.mp4");

    let monitor = match Monitor::primary() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("no primary monitor for screen capture: {e}");
            return None;
        }
    };

    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        ColorFormat::Rgba8,
        path.clone(),
    );

    match Capture::start_free_threaded(settings) {
        Ok(control) => {
            let handle = ScreenHandle::new(Box::new(move || {
                let _ = control.stop();
            }));
            Some((handle, path.replace('\\', "/")))
        }
        Err(e) => {
            tracing::warn!("failed to start screen capture: {e}");
            None
        }
    }
}

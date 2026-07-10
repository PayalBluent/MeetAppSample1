use crate::models::MeetingPlatform;

/// A meeting-like window/process the detector found this scan.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub platform: MeetingPlatform,
    pub title: String,
    pub process_name: String,
}

/// Scan the system for in-progress meetings. Returns at most one candidate per
/// platform. Platform-independent callers use only this function.
pub fn scan() -> Vec<Candidate> {
    #[cfg(windows)]
    {
        return crate::platform::windows::detect::scan();
    }
    #[cfg(target_os = "macos")]
    {
        return crate::platform::macos::detect_scan();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// Representative process/display name for a platform (used when a title-only
/// signal is matched).
pub fn primary_process(platform: MeetingPlatform) -> &'static str {
    match platform {
        MeetingPlatform::GoogleMeet => "chrome.exe",
        MeetingPlatform::Zoom => "Zoom.exe",
        MeetingPlatform::Teams => "ms-teams.exe",
        MeetingPlatform::Discord => "Discord.exe",
        MeetingPlatform::Slack => "slack.exe",
        MeetingPlatform::Webex => "CiscoCollabHost.exe",
        MeetingPlatform::Unknown => "unknown",
    }
}

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

/// Platforms that currently look like they have an **active** meeting, using the
/// lenient continue-check the auto-stop relies on (keeps an in-call window alive
/// even after its title drops the meeting/call token). A superset of [`scan`]'s
/// strict result; never used to *start* a recording. See the Windows
/// implementation for details.
pub fn active_platforms() -> std::collections::HashSet<MeetingPlatform> {
    #[cfg(windows)]
    {
        return crate::platform::windows::detect::active_platforms();
    }
    #[cfg(target_os = "macos")]
    {
        // No lenient in-call heuristic on macOS yet — fall back to the strict scan.
        return crate::platform::macos::detect_scan()
            .into_iter()
            .map(|c| c.platform)
            .collect();
    }
    #[allow(unreachable_code)]
    std::collections::HashSet::new()
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

use std::collections::{HashMap, HashSet};

use sysinfo::System;

use super::window_titles::visible_window_titles;
use crate::models::MeetingPlatform;
use crate::platform::detect::{primary_process, Candidate};

/// Scan visible window titles + running processes and classify in-progress
/// meetings. Detection is title-driven (accurate for Meet/Zoom/Teams) and
/// confirmed against running processes to cut false positives.
pub fn scan() -> Vec<Candidate> {
    let procs = running_process_names();
    let titles = visible_window_titles();

    let mut found: HashMap<MeetingPlatform, Candidate> = HashMap::new();
    for title in titles {
        let lower = title.to_lowercase();
        if let Some(platform) = classify(&lower, &procs) {
            found.entry(platform).or_insert_with(|| Candidate {
                platform,
                title,
                process_name: primary_process(platform).to_string(),
            });
        }
    }
    found.into_values().collect()
}

fn classify(title: &str, procs: &HashSet<String>) -> Option<MeetingPlatform> {
    let has = |name: &str| procs.iter().any(|p| p.contains(name));
    let browser = has("chrome") || has("msedge") || has("firefox") || has("brave");

    // Google Meet (browser tab).
    if title.contains("google meet")
        || (title.contains("meet - ") && browser)
        || (title.contains("meet") && title.contains("meet.google"))
    {
        return Some(MeetingPlatform::GoogleMeet);
    }
    // Zoom shows a dedicated "Zoom Meeting" window only during a call.
    if (title.contains("zoom meeting") || title.contains("zoom.us")) && has("zoom") {
        return Some(MeetingPlatform::Zoom);
    }
    // Teams meeting/call window.
    if title.contains("microsoft teams")
        && (title.contains("meeting") || title.contains("call"))
        && (has("teams") || has("ms-teams"))
    {
        return Some(MeetingPlatform::Teams);
    }
    // Slack huddle.
    if title.contains("huddle") && has("slack") {
        return Some(MeetingPlatform::Slack);
    }
    // Discord voice call (best-effort: title carries the channel while in call).
    if has("discord") && (title.contains("discord") && title.contains(" - ")) {
        return Some(MeetingPlatform::Discord);
    }
    // Webex meeting.
    if title.contains("webex") && title.contains("meeting") {
        return Some(MeetingPlatform::Webex);
    }
    None
}

fn running_process_names() -> HashSet<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().to_lowercase())
        .collect()
}

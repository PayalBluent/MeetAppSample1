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

/// Platforms that currently look like they have an **active** meeting, using a
/// *lenient* check — the one the auto-stop consults to decide whether a running
/// recording should keep going. It is a superset of [`scan`]'s strict result: in
/// addition to the join / pre-call / compact-view windows that carry a
/// "meeting"/"call" token, it treats a Teams **in-call** window (whose title has
/// become the meeting subject and dropped that token) as still-active, so a
/// recording is not cut off mid-call. It returns to *not* active once Teams is back
/// on an idle navigation section (Calendar, Chat, Calls, …), so a finished meeting
/// still auto-stops.
///
/// Recordings are only ever *started* from [`scan`]/`classify` (strict), never from
/// here — so this lenient view cannot cause a false recording, only keep a real one
/// alive through the call.
pub fn active_platforms() -> HashSet<MeetingPlatform> {
    let procs = running_process_names();
    let titles = visible_window_titles();

    let mut set: HashSet<MeetingPlatform> = HashSet::new();
    for title in &titles {
        if let Some(p) = classify(&title.to_lowercase(), &procs) {
            set.insert(p);
        }
    }

    // Lenient Teams continue-check: an in-call window keeps the meeting active even
    // after its title drops the meeting/call token.
    let has = |name: &str| procs.iter().any(|p| p.contains(name));
    if (has("teams") || has("ms-teams"))
        && titles.iter().any(|t| teams_in_call(&t.to_lowercase()))
    {
        set.insert(MeetingPlatform::Teams);
    }
    set
}

/// Whether a (lower-cased) Teams window title looks like an **in-call** window
/// rather than the idle app shell. Used only by [`active_platforms`] (the auto-stop
/// continue-check), never to start a recording. A Teams in-call window is titled
/// with the meeting subject ("&lt;subject&gt; | Microsoft Teams"); the idle app sits
/// on a known navigation section (Calendar, Chat, Calls, …). So a Teams window whose
/// leading segment is a non-empty, non-nav string is treated as in-call.
fn teams_in_call(lower: &str) -> bool {
    let Some(pos) = lower.find("microsoft teams") else {
        return false;
    };
    // Text before "microsoft teams", trailing separators/whitespace stripped.
    let lead = lower[..pos].trim_matches(|c: char| {
        c.is_whitespace() || c == '|' || c == '-' || c == '\u{00b7}'
    });
    if lead.is_empty() {
        return false; // bare "Microsoft Teams" — startup / idle, not a call
    }
    // The section name is the last "| "-separated segment.
    let section = lead
        .rsplit(|c| c == '|' || c == '-' || c == '\u{00b7}')
        .next()
        .unwrap_or(lead)
        .trim();
    // Idle navigation sections — the app is not in a call when it's on one of these.
    const NAV: &[&str] = &[
        "activity", "chat", "calendar", "calls", "teams", "files", "apps", "help",
        "communities", "store", "settings", "more", "home", "shifts", "tasks",
    ];
    !NAV.contains(&section)
}

fn classify(title: &str, procs: &HashSet<String>) -> Option<MeetingPlatform> {
    let has = |name: &str| procs.iter().any(|p| p.contains(name));
    let browser = has("chrome")
        || has("msedge")
        || has("firefox")
        || has("brave")
        || has("opera")
        || has("vivaldi");

    // Google Meet (browser tab). A live call carries a meeting subject or code in
    // the tab title ("<subject> - Google Meet" or "Meet - <code>"). The bare
    // "Google Meet" home page and the post-call "you left the meeting" screen are
    // titled just "Google Meet" (window title "google meet - <browser>"), so we
    // must NOT match those — otherwise a finished call stays "detected" every poll,
    // the auto-stop counter keeps resetting, and the recording never ends. Requiring
    // a subject/code (and a running browser) also drops false matches like a
    // document titled "Google Meet notes".
    if browser
        && (title.contains(" - google meet")
            || title.starts_with("meet - ")
            || title.contains("meet.google"))
    {
        return Some(MeetingPlatform::GoogleMeet);
    }
    // Zoom shows a dedicated "Zoom Meeting" window only during a call.
    if (title.contains("zoom meeting") || title.contains("zoom.us")) && has("zoom") {
        return Some(MeetingPlatform::Zoom);
    }
    // Teams meeting/call window. Exclude the "Calls" navigation tab ("Calls |
    // Microsoft Teams") — that's call *history*, not an active call. It contains
    // the substring "call", so it used to keep a finished meeting detected and
    // block the recording from auto-stopping.
    if title.contains("microsoft teams")
        && (title.contains("meeting") || (title.contains("call") && !title.contains("calls |")))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_teams_meeting_window() {
        // The real in-call window title seen in this user's recordings.
        let title = "meeting compact view - jitesh tripathi - microsoft teams";
        assert_eq!(
            classify(title, &procs(&["ms-teams.exe"])),
            Some(MeetingPlatform::Teams)
        );
    }

    #[test]
    fn ignores_idle_teams_window() {
        // The main Teams window (not in a call) carries no "meeting"/"call" token.
        assert_eq!(classify("chat | microsoft teams", &procs(&["ms-teams.exe"])), None);
    }

    #[test]
    fn detects_google_meet_tab() {
        assert_eq!(
            classify("project sync - google meet", &procs(&["chrome.exe"])),
            Some(MeetingPlatform::GoogleMeet)
        );
    }

    #[test]
    fn detects_google_meet_code_tab() {
        // A meeting joined by code: the tab title begins "Meet - <code>".
        assert_eq!(
            classify("meet - abc-defg-hij - google chrome", &procs(&["chrome.exe"])),
            Some(MeetingPlatform::GoogleMeet)
        );
    }

    #[test]
    fn ignores_google_meet_home_screen() {
        // The Meet landing / "you left the meeting" screen is titled just
        // "Google Meet" (with Chrome's window suffix). It is NOT an active call, so
        // it must not keep a finished meeting detected and block auto-stop.
        assert_eq!(
            classify("google meet - google chrome", &procs(&["chrome.exe"])),
            None
        );
    }

    #[test]
    fn ignores_teams_calls_tab() {
        // The "Calls" navigation tab is call history, not an active call.
        assert_eq!(
            classify("calls | microsoft teams", &procs(&["ms-teams.exe"])),
            None
        );
    }

    #[test]
    fn detects_google_meet_with_browser_window_suffix() {
        // Real Chrome window titles append " - Google Chrome"; a live call still has
        // a subject before "Google Meet", so it must be detected.
        assert_eq!(
            classify("project sync - google meet - google chrome", &procs(&["chrome.exe"])),
            Some(MeetingPlatform::GoogleMeet)
        );
    }

    #[test]
    fn ignores_zoom_home_window() {
        // Zoom's home window ("Zoom Workplace") is not an active call.
        assert_eq!(classify("zoom workplace", &procs(&["zoom.exe"])), None);
    }

    #[test]
    fn teams_in_call_keeps_subject_window_active() {
        // In-call windows are titled with the meeting subject (no meeting/call
        // token) — the auto-stop must still treat these as an active meeting.
        assert!(teams_in_call("weekly sync | microsoft teams"));
        assert!(teams_in_call("q3 planning - roadmap | microsoft teams"));
        assert!(teams_in_call("meeting compact view - jitesh - microsoft teams"));
    }

    #[test]
    fn teams_in_call_treats_idle_nav_as_ended() {
        // Idle navigation sections mean no active call — the recording should stop.
        for t in [
            "calendar | microsoft teams",
            "chat | microsoft teams",
            "calls | microsoft teams",
            "activity | microsoft teams",
            "microsoft teams", // bare startup window
        ] {
            assert!(!teams_in_call(t), "should be idle: {t}");
        }
    }

    #[test]
    fn no_match_without_meeting_process() {
        // A title mentioning Teams but with no Teams process running is not a call.
        assert_eq!(classify("microsoft teams meeting notes.docx", &procs(&["winword.exe"])), None);
    }
}

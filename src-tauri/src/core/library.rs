//! On-disk recording library: metadata persistence + folder import.
//!
//! Recordings are media files in the user's recordings folder; their metadata
//! (title, transcript, participants, summary, flags, timestamps) is persisted as a
//! JSON **sidecar** per recording under `<recordings>/metadata/<id>.json`. This is
//! what makes recordings survive an app restart — without it the in-memory meeting
//! list is rebuilt from demo seeds every launch and real recordings, though still
//! on disk, are invisible.
//!
//! On startup [`load_all`] reads every sidecar and also **imports** any media file
//! that has no sidecar yet (recordings made before this feature existed, or copied
//! in by hand), writing a sidecar for it so nothing in the folder stays hidden.
//!
//! Nothing here ever deletes or overwrites a *media* file except [`delete`], which
//! is only reached from the explicit "delete meeting" command. [`save`] only ever
//! writes sidecar JSON, and does so atomically so a crash can't corrupt one.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::models::{
    CaptureMode, Meeting, MeetingPlatform, MeetingStatus, TimelineKind, TimelineMarker,
};

/// Subfolder (under the recordings dir) holding one JSON sidecar per recording.
/// Keeping sidecars out of the recordings dir itself leaves that folder clean —
/// just the media the user expects to see.
const META_DIR: &str = "metadata";

fn meta_dir(recordings_dir: &Path) -> PathBuf {
    recordings_dir.join(META_DIR)
}

fn sidecar_path(recordings_dir: &Path, id: &str) -> PathBuf {
    meta_dir(recordings_dir).join(format!("{id}.json"))
}

/// Whether a meeting represents a real recording worth persisting — it has a media
/// file. Demo/seed meetings have no media and are never written to disk.
fn has_media(m: &Meeting) -> bool {
    m.audio_path.is_some() || m.video_path.is_some()
}

/// Persist a meeting's metadata sidecar. No-op for meetings without media (so demo
/// seeds are never written). Writes atomically (temp file + rename) so a crash
/// mid-write can't corrupt an existing sidecar. Failures are logged, never
/// propagated — persistence must never break a recording or a UI action.
pub fn save(recordings_dir: &Path, m: &Meeting) {
    if !has_media(m) {
        return;
    }
    let dir = meta_dir(recordings_dir);
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("library: cannot create metadata dir {dir:?}: {e}");
        return;
    }
    let json = match serde_json::to_vec_pretty(m) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("library: cannot serialize meeting {}: {e}", m.id);
            return;
        }
    };
    let final_path = sidecar_path(recordings_dir, &m.id);
    // Per-thread temp name so two concurrent persists of the same meeting (e.g. a
    // rename racing the finalize thread) never write the same temp file. Each
    // rename is atomic, so the final sidecar is always one writer's complete JSON.
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let tmp_path = dir.join(format!("{}.{tid}.json.tmp", m.id));
    if fs::write(&tmp_path, &json).is_ok() && fs::rename(&tmp_path, &final_path).is_ok() {
        return;
    }
    // Fallback: rename can fail on some filesystems — write in place instead.
    let _ = fs::remove_file(&tmp_path);
    if let Err(e) = fs::write(&final_path, &json) {
        tracing::warn!("library: cannot write sidecar {final_path:?}: {e}");
    }
}

/// Delete a meeting's media files and its sidecar. Only reached from the explicit
/// user "delete" command — this is the one place the library removes recordings.
/// Missing files are ignored.
pub fn delete(recordings_dir: &Path, m: &Meeting) {
    for p in [m.audio_path.as_ref(), m.video_path.as_ref()]
        .into_iter()
        .flatten()
    {
        if Path::new(p).exists() {
            if let Err(e) = fs::remove_file(p) {
                tracing::warn!("library: could not delete media {p}: {e}");
            }
        }
    }
    let _ = fs::remove_file(sidecar_path(recordings_dir, &m.id));
    tracing::info!("library: deleted recording {}", m.id);
}

/// Load every persisted recording, importing any media file that has no sidecar.
/// Returns the meetings to seed the in-memory store with on startup. Never fails:
/// unreadable sidecars/files are skipped with a warning.
pub fn load_all(recordings_dir: &Path) -> Vec<Meeting> {
    let mut out: Vec<Meeting> = Vec::new();
    // Media filenames already owned by a loaded meeting — so we don't re-import them.
    let mut referenced: HashSet<String> = HashSet::new();

    // 1) Load sidecars.
    if let Ok(entries) = fs::read_dir(meta_dir(recordings_dir)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Meeting>(&b).ok())
            {
                Some(mut m) => {
                    // A recording exists on disk, so a persisted *transient* status
                    // (crashed mid-record or mid-process) is treated as finished.
                    if matches!(m.status, MeetingStatus::Live | MeetingStatus::Processing) {
                        m.status = MeetingStatus::Ready;
                    }
                    for name in media_names(&m) {
                        referenced.insert(name);
                    }
                    out.push(m);
                }
                None => tracing::warn!("library: skipping unreadable sidecar {path:?}"),
            }
        }
    }

    // 2) Import orphan media — any .wav in the recordings dir without a sidecar.
    if let Ok(entries) = fs::read_dir(recordings_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_wav = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("wav"))
                .unwrap_or(false);
            if !is_wav {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if referenced.contains(&name) {
                continue; // already owned by a loaded meeting
            }
            let m = import_orphan(&path);
            // Write a sidecar now so future launches load it directly and re-scans
            // never re-import (and thus never duplicate) it.
            save(recordings_dir, &m);
            referenced.insert(name);
            out.push(m);
        }
    }

    tracing::info!(
        "library: loaded {} recording(s) from {recordings_dir:?}",
        out.len()
    );
    out
}

/// Basenames of the media files a meeting references.
fn media_names(m: &Meeting) -> Vec<String> {
    [m.audio_path.as_ref(), m.video_path.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
        })
        .collect()
}

/// Build a Meeting from a bare media file that has no sidecar. The id is derived
/// from the filename so re-scans are stable — the same file always maps to the same
/// meeting, never a duplicate across restarts.
fn import_orphan(path: &Path) -> Meeting {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording")
        .to_string();
    let id = derive_id(&stem);
    let title = derive_title(&stem);
    let started_at = file_time(path);
    let duration_sec = wav_duration_sec(path);
    let audio_path = path.to_string_lossy().replace('\\', "/");

    Meeting {
        id,
        title,
        platform: MeetingPlatform::Unknown,
        mode: CaptureMode::Record,
        status: MeetingStatus::Ready,
        started_at,
        ended_at: Some(started_at + chrono::Duration::seconds(duration_sec as i64)),
        duration_sec,
        has_audio: true,
        has_video: false,
        is_locked: false,
        is_starred: false,
        is_bookmarked: false,
        tags: Vec::new(),
        participants: Vec::new(),
        timeline: vec![TimelineMarker {
            id: format!("t_{}", uuid::Uuid::new_v4().simple()),
            label: "Recording started".into(),
            at_ms: 0,
            kind: TimelineKind::Join,
        }],
        transcript: Vec::new(),
        summary: None,
        action_items: Vec::new(),
        audio_path: Some(audio_path),
        video_path: None,
    }
}

/// A stable id for an imported file. Recorder files are `<slug>-mtg_<hex>.wav`, so
/// reuse that `mtg_<hex>` when present; otherwise derive a stable id from the stem.
fn derive_id(stem: &str) -> String {
    if let Some(pos) = stem.rfind("mtg_") {
        return stem[pos..].to_string();
    }
    let slug: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("imported_{slug}")
}

/// A readable title from the filename: drop the trailing `-mtg_<hex>` id, turn
/// separators into spaces, and collapse/trim whitespace.
fn derive_title(stem: &str) -> String {
    let base = match stem.rfind("-mtg_") {
        Some(pos) => &stem[..pos],
        None => stem,
    };
    let spaced: String = base
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let title = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "Recording".to_string()
    } else {
        title
    }
}

/// The file's last-modified time as a UTC timestamp; falls back to now if it can't
/// be read (so an imported recording still gets a sensible sort key).
fn file_time(path: &Path) -> DateTime<Utc> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
}

/// Duration of a WAV in whole seconds from its header; 0 if it can't be read.
fn wav_duration_sec(path: &Path) -> u64 {
    match hound::WavReader::open(path) {
        Ok(r) => {
            let spec = r.spec();
            let frames = r.len() as u64 / spec.channels.max(1) as u64;
            if spec.sample_rate > 0 {
                frames / spec.sample_rate as u64
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("meetapp-library-test").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path, secs: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(48_000 * secs) {
            w.write_sample(((i as f32 * 0.05).sin() * 0.3 * i16::MAX as f32) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
    }

    fn recording(dir: &Path, id: &str, title: &str) -> Meeting {
        let wav = dir.join(format!("{title}-{id}.wav"));
        write_wav(&wav, 1);
        let mut m = Meeting::new_live(title, MeetingPlatform::Unknown, CaptureMode::Record);
        m.id = id.to_string();
        m.audio_path = Some(wav.to_string_lossy().replace('\\', "/"));
        m
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tmp_dir("roundtrip");
        let mut m = recording(&dir, "mtg_round", "Quarterly sync");
        m.is_starred = true;
        save(&dir, &m);

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "mtg_round");
        assert_eq!(loaded[0].title, "Quarterly sync");
        assert!(loaded[0].is_starred, "flags must round-trip");
    }

    #[test]
    fn rename_persists_across_reload() {
        let dir = tmp_dir("rename");
        let mut m = recording(&dir, "mtg_rename", "Old title");
        save(&dir, &m);
        // Rename = change the title and re-persist the same sidecar.
        m.title = "New title".to_string();
        save(&dir, &m);

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1, "no duplicate sidecar after rename");
        assert_eq!(loaded[0].id, "mtg_rename");
        assert_eq!(loaded[0].title, "New title", "renamed title survives reload");
    }

    #[test]
    fn does_not_persist_meeting_without_media() {
        let dir = tmp_dir("no-media");
        let m = Meeting::new_live("Demo", MeetingPlatform::Unknown, CaptureMode::Record);
        // new_live has no audio_path/video_path.
        save(&dir, &m);
        assert!(load_all(&dir).is_empty(), "media-less meetings aren't persisted");
    }

    #[test]
    fn imports_orphan_wav_with_a_sidecar() {
        let dir = tmp_dir("orphan");
        // A bare recording file, no sidecar (e.g. made before this feature).
        write_wav(&dir.join("team-standup-mtg_orphan1.wav"), 2);

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        let m = &loaded[0];
        assert_eq!(m.id, "mtg_orphan1", "id derived from filename");
        assert_eq!(m.title, "team standup", "title derived from filename");
        assert!(m.has_audio && m.audio_path.is_some());
        assert!(m.duration_sec >= 1, "duration read from WAV header");
        // A sidecar was written so it loads directly next time.
        assert!(sidecar_path(&dir, "mtg_orphan1").exists());
    }

    #[test]
    fn rescan_does_not_duplicate_imported_recordings() {
        let dir = tmp_dir("rescan");
        write_wav(&dir.join("chat-mtg_dup.wav"), 1);

        let first = load_all(&dir);
        let second = load_all(&dir);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1, "stable id → no duplicate on the second scan");
        assert_eq!(second[0].id, "mtg_dup");
    }

    #[test]
    fn delete_removes_media_and_sidecar() {
        let dir = tmp_dir("delete");
        let m = recording(&dir, "mtg_del", "Throwaway");
        save(&dir, &m);
        let audio = m.audio_path.clone().unwrap();
        assert!(Path::new(&audio).exists());
        assert!(sidecar_path(&dir, "mtg_del").exists());

        delete(&dir, &m);

        assert!(!Path::new(&audio).exists(), "media file removed on explicit delete");
        assert!(!sidecar_path(&dir, "mtg_del").exists(), "sidecar removed");
        assert!(load_all(&dir).is_empty(), "nothing re-imported after delete");
    }

    #[test]
    fn transient_status_becomes_ready_on_load() {
        let dir = tmp_dir("transient");
        let mut m = recording(&dir, "mtg_live", "Interrupted");
        m.status = MeetingStatus::Live; // simulate a crash mid-recording
        save(&dir, &m);

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        assert!(matches!(loaded[0].status, MeetingStatus::Ready));
    }
}

/**
 * Shared domain types.
 *
 * These mirror the Rust models in `src-tauri/src/models` exactly (field names use
 * camelCase on both sides via serde `rename_all = "camelCase"`). Keep the two in
 * sync — the Tauri bridge deserializes command results straight into these.
 */

/** Meeting platforms the detector recognises. */
export type MeetingPlatform =
  | "googleMeet"
  | "zoom"
  | "teams"
  | "discord"
  | "slack"
  | "webex"
  | "unknown";

/** What the AI Note Taker is set to do for the next / current meeting. */
export type CaptureMode = "off" | "transcribe" | "record" | "recordVideo";

/** Lifecycle of the recorder subsystem. */
export type RecorderState =
  | "idle" // nothing happening
  | "armed" // a mode is selected, waiting for a meeting / manual start
  | "detecting" // a meeting was detected, prompting / auto-capturing
  | "recording" // actively capturing
  | "processing" // finalising: transcribing / summarising
  | "error";

/** Status of a stored meeting record. */
export type MeetingStatus = "live" | "processing" | "ready" | "failed";

export interface TranscriptSegment {
  id: string;
  /** Speaker label (diarization) — "Speaker 1", or a resolved participant name. */
  speaker: string;
  text: string;
  /** Offset from the start of the meeting, in milliseconds. */
  startMs: number;
  endMs: number;
  /** Model confidence 0..1, when available. */
  confidence?: number;
}

export interface ActionItem {
  id: string;
  text: string;
  done: boolean;
  assignee?: string | null;
  /** ISO date string. */
  dueDate?: string | null;
}

export interface MeetingSummary {
  /** One-paragraph overview. */
  tldr: string;
  keyPoints: string[];
  decisions: string[];
  /** ISO timestamp of when the summary was generated. */
  generatedAt: string;
  /** Which model / provider produced it (for provenance in the UI). */
  model: string;
}

export interface Participant {
  id: string;
  name: string;
  /** Fraction of talk time 0..1, when available. */
  talkRatio?: number;
}

/** A timeline marker (chapter, highlight, action-item anchor). */
export interface TimelineMarker {
  id: string;
  label: string;
  atMs: number;
  kind: "chapter" | "highlight" | "action" | "join" | "leave";
}

export interface Meeting {
  id: string;
  title: string;
  platform: MeetingPlatform;
  mode: CaptureMode;
  status: MeetingStatus;

  /** ISO timestamps. */
  startedAt: string;
  endedAt?: string | null;
  durationSec: number;

  hasAudio: boolean;
  hasVideo: boolean;

  isLocked: boolean;
  isStarred: boolean;
  isBookmarked: boolean;

  tags: string[];
  participants: Participant[];
  timeline: TimelineMarker[];

  transcript: TranscriptSegment[];
  summary?: MeetingSummary | null;
  actionItems: ActionItem[];

  /** Absolute paths on disk when recordings were saved. */
  audioPath?: string | null;
  videoPath?: string | null;
}

/** A meeting the detector currently believes is in progress. */
export interface DetectedMeeting {
  id: string;
  platform: MeetingPlatform;
  title: string;
  /** Owning process, e.g. "Zoom.exe", "ms-teams.exe", "chrome.exe". */
  processName: string;
  detectedAt: string;
  /** True once we've started capturing this detection. */
  capturing: boolean;
}

export interface Settings {
  defaultMode: CaptureMode;
  autoRecordDetected: boolean;
  /** Record the computer's audio output (speaker) alongside the mic. */
  captureSystemAudio: boolean;
  cancelMyNoise: boolean;
  cancelOthersNoise: boolean;
  launchOnStartup: boolean;
  startMinimized: boolean;
  /** Directory recordings are written to. */
  saveDirectory: string;
  theme: "light" | "dark" | "system";
  /** Cloud AI credentials (also read from env vars when blank). */
  assemblyaiApiKey: string;
  groqApiKey: string;
  groqModel: string;
}

/** Aggregate recorder status streamed to the UI. */
export interface RecorderStatus {
  state: RecorderState;
  mode: CaptureMode;
  /** id of the meeting currently being recorded, if any. */
  activeMeetingId?: string | null;
  /** Seconds elapsed in the current capture. */
  elapsedSec: number;
  /** Live input levels 0..1 for the meters. */
  micLevel: number;
  systemLevel: number;
  message?: string | null;
}

/** Health of the Windows shared-mode audio engine (see `audio_health` command). */
export interface AudioHealth {
  /** False on non-Windows platforms (the shared-mode APO issue is Windows-only). */
  supported: boolean;
  /** Shared-mode capture works — the normal, healthy path. */
  sharedOk: boolean;
  /** Exclusive-mode mic works — capture still possible even if shared mode is broken. */
  exclusiveOk: boolean;
  /** Shared mode is impaired; the one-click repair is recommended. */
  needsRepair: boolean;
  /** Human-readable one-line summary. */
  detail: string;
}

/** Event payloads emitted by the Rust backend (see `src-tauri/src/events.rs`). */
export interface AppEvents {
  "meeting://detected": DetectedMeeting;
  "meeting://ended": { id: string };
  "recorder://status": RecorderStatus;
  "recorder://transcript": { meetingId: string; segment: TranscriptSegment };
  "meeting://updated": Meeting;
}

export type AppEventName = keyof AppEvents;

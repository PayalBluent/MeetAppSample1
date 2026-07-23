import type {
  AudioHealth,
  CaptureMode,
  DetectedMeeting,
  Meeting,
  MeetingPlatform,
  RecorderStatus,
  Settings,
} from "@/types";
import { invoke } from "./tauri";

/**
 * Typed façade over every backend command. UI code should call these — never
 * `invoke` directly — so the command surface stays discoverable and typed.
 */
export const api = {
  listMeetings: () => invoke<Meeting[]>("get_meetings"),
  getMeeting: (id: string) => invoke<Meeting | null>("get_meeting", { id }),

  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: Partial<Settings>) =>
    invoke<Settings>("update_settings", { patch }),

  getRecorderStatus: () => invoke<RecorderStatus>("get_recorder_status"),
  getDetectedMeetings: () => invoke<DetectedMeeting[]>("get_detected_meetings"),

  setMode: (mode: CaptureMode) => invoke<RecorderStatus>("set_mode", { mode }),

  startCapture: (opts?: {
    title?: string;
    platform?: MeetingPlatform;
    meetingId?: string;
  }) => invoke<RecorderStatus>("start_capture", opts ?? {}),

  stopCapture: () => invoke<Meeting | null>("stop_capture"),

  /** Set the capture volume (input gain). Takes effect live during recording. */
  setInputGain: (gain: number) =>
    invoke<RecorderStatus>("set_input_gain", { gain }),

  /**
   * Mute/unmute the microphone for the recording. Use this when you mute yourself
   * inside a meeting app (Teams/Zoom/Meet) — those in-app mutes can't be detected
   * automatically, so this stops your mic from being recorded. System audio keeps
   * recording. Takes effect within a quarter-second.
   */
  setMicMute: (muted: boolean) =>
    invoke<RecorderStatus>("set_mic_mute", { muted }),

  captureDetected: (id: string) =>
    invoke<RecorderStatus>("capture_detected", { id }),
  dismissDetected: (id: string) => invoke<void>("dismiss_detected", { id }),

  sendBot: (url?: string) => invoke<{ ok: boolean }>("send_bot", { url }),

  toggleFlag: (id: string, flag: "locked" | "starred" | "bookmarked") =>
    invoke<Meeting>("toggle_meeting_flag", { id, flag }),

  renameMeeting: (id: string, title: string) =>
    invoke<Meeting>("rename_meeting", { id, title }),

  updateActionItem: (meetingId: string, itemId: string, done: boolean) =>
    invoke<Meeting>("update_action_item", { meetingId, itemId, done }),

  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),

  /** On-demand transcription via AssemblyAI. */
  transcribeMeeting: (id: string) =>
    invoke<Meeting>("transcribe_meeting", { id }),

  /** On-demand summarization via Groq (heuristic fallback). */
  summarizeMeeting: (id: string) =>
    invoke<Meeting>("summarize_meeting", { id }),

  /** Loudness-normalize a saved recording so it's clearly audible (no clipping). */
  enhanceMeetingAudio: (id: string) =>
    invoke<Meeting>("enhance_meeting_audio", { id }),

  /** AI noise-cancel a saved recording (RNNoise) in place. */
  cleanMeetingAudio: (id: string) =>
    invoke<Meeting>("clean_meeting_audio", { id }),

  openRecordingsFolder: () => invoke<void>("open_recordings_folder"),

  /** Probe the Windows shared-mode audio engine (healthy vs. needs repair). */
  audioHealth: () => invoke<AudioHealth>("audio_health"),
  /** Disable the broken audio enhancement + restart Windows Audio (elevated). */
  repairAudio: () => invoke<string>("repair_audio"),
  /** Open the classic Windows Sound control panel for the manual fix. */
  openSoundSettings: () => invoke<void>("open_sound_settings"),
};

export type Api = typeof api;

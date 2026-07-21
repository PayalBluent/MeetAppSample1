import type {
  ActionItem,
  CaptureMode,
  DetectedMeeting,
  Meeting,
  MeetingSummary,
  RecorderState,
  RecorderStatus,
  Settings,
  TranscriptSegment,
} from "@/types";
import { EventBus } from "./eventBus";
import { createSeedMeetings, nextId } from "./seed";

/** Mirrors the backend message shown when an audio action has no audio to act on. */
const AUDIO_UNAVAILABLE =
  "Audio isn't available for this meeting, so it can't be enhanced or cleaned yet.";

/**
 * In-memory stand-in for the Rust backend. It implements the exact same command
 * surface and event stream, so the React app is fully functional in a plain
 * browser (no Tauri runtime) and during Vite development.
 */
class MockBackend {
  readonly bus = new EventBus();

  private meetings = new Map<string, Meeting>();
  private detected = new Map<string, DetectedMeeting>();
  private settings: Settings = {
    defaultMode: "off",
    autoRecordDetected: true,
    captureSystemAudio: true,
    cancelMyNoise: true,
    cancelOthersNoise: true,
    launchOnStartup: false,
    startMinimized: true,
    saveDirectory: "C:/Users/you/MeetApp/recordings",
    theme: "light",
    assemblyaiApiKey: "",
    groqApiKey: "",
    groqModel: "llama-3.3-70b-versatile",
  };
  private status: RecorderStatus = {
    state: "idle",
    mode: "off",
    activeMeetingId: null,
    elapsedSec: 0,
    micLevel: 0,
    systemLevel: 0,
    inputGain: 1.5,
    audioReady: false,
    message: null,
  };

  private tickTimer: ReturnType<typeof setInterval> | null = null;
  private transcriptTimer: ReturnType<typeof setInterval> | null = null;
  private detectTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    for (const m of createSeedMeetings()) this.meetings.set(m.id, m);
  }

  /** Single entry point mirroring Tauri's `invoke(cmd, args)`. */
  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    // Tiny latency so loading states are exercised realistically.
    await delay(60);
    const handler = (this.handlers as Record<string, (a: any) => unknown>)[cmd];
    if (!handler) throw new Error(`[mock] unknown command: ${cmd}`);
    return handler.call(this, args ?? {}) as T;
  }

  // ----------------------------------------------------------------- commands
  private handlers = {
    get_meetings: (): Meeting[] =>
      [...this.meetings.values()].sort(
        (a, b) => +new Date(b.startedAt) - +new Date(a.startedAt),
      ),

    get_meeting: ({ id }: { id: string }): Meeting | null =>
      this.meetings.get(id) ?? null,

    get_settings: (): Settings => ({ ...this.settings }),

    update_settings: ({ patch }: { patch: Partial<Settings> }): Settings => {
      this.settings = { ...this.settings, ...patch };
      return { ...this.settings };
    },

    get_recorder_status: (): RecorderStatus => ({ ...this.status }),

    get_detected_meetings: (): DetectedMeeting[] => [...this.detected.values()],

    set_mode: ({ mode }: { mode: CaptureMode }): RecorderStatus => {
      this.settings.defaultMode = mode;
      this.status.mode = mode;

      if (mode === "off") {
        this.clearDetectionSchedule();
        // Emit `ended` for every open detection before clearing so the UI drops
        // the cards (mirrors the real backend's detection loop on Off).
        for (const id of this.detected.keys()) this.bus.emit("meeting://ended", { id });
        this.detected.clear();
        if (this.status.state !== "recording") this.setState("idle");
      } else if (this.status.state === "idle") {
        this.setState("armed");
        this.scheduleFakeDetection();
      }
      this.emitStatus();
      return { ...this.status };
    },

    start_capture: ({
      title,
      platform,
      meetingId,
    }: {
      title?: string;
      platform?: Meeting["platform"];
      meetingId?: string;
    }): RecorderStatus => {
      // "Record Live" is an explicit manual action — always records, starting a
      // Record capture even when the mode is Off (mirrors the real backend).
      const mode = this.status.mode === "off" ? "record" : this.status.mode;
      this.beginLiveMeeting({
        title: title ?? "Live recording",
        platform: platform ?? "unknown",
        mode,
        fromDetectionId: meetingId,
      });
      return { ...this.status };
    },

    stop_capture: (): Meeting | null => this.finalizeLiveMeeting(),

    set_input_gain: ({ gain }: { gain: number }): RecorderStatus => {
      const clamped = Number.isFinite(gain) ? Math.min(3, Math.max(0, gain)) : 1;
      this.status.inputGain = clamped;
      this.emitStatus();
      return { ...this.status };
    },

    capture_detected: ({ id }: { id: string }): RecorderStatus => {
      const det = this.detected.get(id);
      if (det) {
        det.capturing = true;
        this.beginLiveMeeting({
          title: det.title,
          platform: det.platform,
          mode: this.status.mode === "off" ? "record" : this.status.mode,
          fromDetectionId: id,
        });
      }
      return { ...this.status };
    },

    dismiss_detected: ({ id }: { id: string }): void => {
      this.detected.delete(id);
    },

    send_bot: ({ url }: { url?: string }): { ok: boolean } => {
      // In the real backend this dispatches a meeting bot. Here we just ack.
      console.info("[mock] send_bot", url);
      return { ok: true };
    },

    toggle_meeting_flag: ({
      id,
      flag,
    }: {
      id: string;
      flag: "locked" | "starred" | "bookmarked";
    }): Meeting => {
      const m = this.requireMeeting(id);
      if (flag === "locked") m.isLocked = !m.isLocked;
      if (flag === "starred") m.isStarred = !m.isStarred;
      if (flag === "bookmarked") m.isBookmarked = !m.isBookmarked;
      this.bus.emit("meeting://updated", { ...m });
      return { ...m };
    },

    rename_meeting: ({ id, title }: { id: string; title: string }): Meeting => {
      const m = this.requireMeeting(id);
      m.title = title;
      this.bus.emit("meeting://updated", { ...m });
      return { ...m };
    },

    update_action_item: ({
      meetingId,
      itemId,
      done,
    }: {
      meetingId: string;
      itemId: string;
      done: boolean;
    }): Meeting => {
      const m = this.requireMeeting(meetingId);
      m.actionItems = m.actionItems.map((it) =>
        it.id === itemId ? { ...it, done } : it,
      );
      this.bus.emit("meeting://updated", { ...m });
      return { ...m };
    },

    delete_meeting: ({ id }: { id: string }): void => {
      this.meetings.delete(id);
    },

    transcribe_meeting: ({ id }: { id: string }): Meeting => {
      const m = this.requireMeeting(id);
      if (m.transcript.length === 0) {
        m.transcript = LIVE_LINES.map((l, i) => ({
          id: nextId("seg"),
          speaker: l.speaker,
          text: l.text,
          startMs: i * 4_000,
          endMs: i * 4_000 + 3_500,
          confidence: 0.95,
        }));
        const names = [...new Set(m.transcript.map((s) => s.speaker))];
        m.participants = names.map((n) => ({
          id: nextId("p"),
          name: n,
          talkRatio: 1 / names.length,
        }));
      }
      m.status = "ready";
      this.bus.emit("meeting://updated", { ...m });
      return { ...m };
    },

    summarize_meeting: ({ id }: { id: string }): Meeting => {
      const m = this.requireMeeting(id);
      if (m.transcript.length === 0) {
        throw new Error("Transcribe the meeting first, then summarize.");
      }
      m.summary = this.summarize(m.transcript, m.title);
      m.actionItems = this.extractActionItems(m.transcript);
      m.status = "ready";
      this.bus.emit("meeting://updated", { ...m });
      return { ...m };
    },

    enhance_meeting_audio: ({ id }: { id: string }): Meeting => {
      const m = this.requireMeeting(id);
      if (!m.hasAudio) throw new Error(AUDIO_UNAVAILABLE);
      // Real backend loudness-normalizes the WAV in place; the mock just echoes.
      console.info("[mock] enhance_meeting_audio", id);
      return { ...m };
    },

    clean_meeting_audio: ({ id }: { id: string }): Meeting => {
      const m = this.requireMeeting(id);
      if (!m.hasAudio) throw new Error(AUDIO_UNAVAILABLE);
      // Real backend runs RNNoise over the WAV in place; the mock just echoes.
      console.info("[mock] clean_meeting_audio", id);
      return { ...m };
    },

    open_recordings_folder: (): void => {
      console.info("[mock] open_recordings_folder", this.settings.saveDirectory);
    },

    audio_health: () => ({
      supported: true,
      sharedOk: true,
      exclusiveOk: true,
      needsRepair: false,
      detail:
        "Shared-mode audio is healthy — the microphone and system audio record normally.",
    }),

    repair_audio: (): string => {
      console.info("[mock] repair_audio");
      return "Audio repair launched (mock).";
    },

    open_sound_settings: (): void => {
      console.info("[mock] open_sound_settings");
    },
  };

  // ------------------------------------------------------------- simulation
  private scheduleFakeDetection() {
    this.clearDetectionSchedule();
    this.detectTimer = setTimeout(() => {
      if (this.status.state !== "armed") return;
      const sample = SAMPLE_DETECTIONS[this.detected.size % SAMPLE_DETECTIONS.length]!;
      const det: DetectedMeeting = {
        id: nextId("det"),
        platform: sample.platform,
        title: sample.title,
        processName: sample.processName,
        detectedAt: new Date().toISOString(),
        capturing: false,
      };
      this.detected.set(det.id, det);
      this.setState("detecting");
      this.bus.emit("meeting://detected", det);
      this.emitStatus();

      if (this.settings.autoRecordDetected) {
        setTimeout(() => {
          if (this.detected.has(det.id) && this.status.state !== "recording") {
            det.capturing = true;
            this.beginLiveMeeting({
              title: det.title,
              platform: det.platform,
              mode: this.status.mode === "off" ? "record" : this.status.mode,
              fromDetectionId: det.id,
            });
          }
        }, 2_200);
      }
    }, 6_000);
  }

  private clearDetectionSchedule() {
    if (this.detectTimer) clearTimeout(this.detectTimer);
    this.detectTimer = null;
  }

  private beginLiveMeeting(opts: {
    title: string;
    platform: Meeting["platform"];
    mode: CaptureMode;
    fromDetectionId?: string;
  }) {
    if (this.status.state === "recording") return;
    this.clearDetectionSchedule();

    const now = new Date().toISOString();
    const meeting: Meeting = {
      id: nextId("mtg"),
      title: opts.title,
      platform: opts.platform,
      mode: opts.mode,
      status: "live",
      startedAt: now,
      endedAt: null,
      durationSec: 0,
      hasAudio: opts.mode === "record" || opts.mode === "recordVideo",
      hasVideo: opts.mode === "recordVideo",
      isLocked: false,
      isStarred: false,
      isBookmarked: false,
      tags: [],
      participants: [{ id: nextId("p"), name: "You", talkRatio: 1 }],
      timeline: [{ id: nextId("t"), label: "Recording started", atMs: 0, kind: "join" }],
      transcript: [],
      actionItems: [],
      summary: null,
      audioPath: null,
      videoPath: null,
    };
    this.meetings.set(meeting.id, meeting);

    this.status.state = "recording";
    this.status.mode = opts.mode; // reflect the effective capture mode in the UI
    this.status.activeMeetingId = meeting.id;
    this.status.elapsedSec = 0;
    // Capture isn't live instantly: real devices need a moment to open (WASAPI
    // activation, mic validation). Mirror that so the "starting…" UI is exercised
    // in dev, then flip to live after a short delay.
    this.status.audioReady = false;
    this.status.micLevel = 0;
    this.status.systemLevel = 0;
    this.emitStatus();
    this.bus.emit("meeting://updated", { ...meeting });

    setTimeout(() => {
      if (this.status.activeMeetingId !== meeting.id) return; // stopped meanwhile
      this.status.audioReady = true;
      this.emitStatus();
    }, 1_200);

    // 1s heartbeat: elapsed + animated input levels (levels stay at 0 until live).
    this.tickTimer = setInterval(() => {
      this.status.elapsedSec += 1;
      meeting.durationSec = this.status.elapsedSec;
      if (!this.status.audioReady) return;
      // Scale the simulated levels by the capture volume so the meters visibly
      // respond to the volume control (mirrors the real gain-boosted meter).
      const g = this.status.inputGain;
      const base = 0.25 + Math.random() * 0.4;
      const sysBase =
        opts.mode === "transcribe" ? 0.15 + Math.random() * 0.2 : 0.3 + Math.random() * 0.35;
      this.status.micLevel = Math.min(1, base * g);
      this.status.systemLevel = Math.min(1, sysBase * g);
      this.emitStatus();
    }, 1_000);

    // No live transcript: transcription is on-demand (or auto at stop for
    // Transcribe mode). See finalizeLiveMeeting.
  }

  private finalizeLiveMeeting(): Meeting | null {
    const id = this.status.activeMeetingId;
    if (!id) return null;
    const meeting = this.meetings.get(id);
    if (!meeting) return null;

    if (this.tickTimer) clearInterval(this.tickTimer);
    if (this.transcriptTimer) clearInterval(this.transcriptTimer);
    this.tickTimer = this.transcriptTimer = null;

    this.setState("processing");
    this.status.micLevel = 0;
    this.status.systemLevel = 0;
    this.status.audioReady = false;
    this.emitStatus();

    meeting.endedAt = new Date().toISOString();
    meeting.status = "processing";
    if (meeting.hasAudio) {
      const slug = meeting.title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
      meeting.audioPath = `${this.settings.saveDirectory}/${slug || "recording"}.wav`;
      if (meeting.hasVideo) meeting.videoPath = `${this.settings.saveDirectory}/${slug || "recording"}.mp4`;
    }

    // Finalize latency. Transcribe mode auto-transcribes and keeps no audio;
    // Record / RecordVideo keep the audio with transcript + summary on demand.
    setTimeout(() => {
      if (meeting.mode === "transcribe") {
        meeting.transcript = LIVE_LINES.map((l, i) => ({
          id: nextId("seg"),
          speaker: l.speaker,
          text: l.text,
          startMs: i * 4_000,
          endMs: i * 4_000 + 3_500,
          confidence: 0.95,
        }));
        const names = [...new Set(meeting.transcript.map((s) => s.speaker))];
        meeting.participants = names.map((n) => ({
          id: nextId("p"),
          name: n,
          talkRatio: 1 / names.length,
        }));
        meeting.audioPath = null;
        meeting.hasAudio = false;
      }
      meeting.status = "ready";
      this.bus.emit("meeting://updated", { ...meeting });

      this.status.activeMeetingId = null;
      this.status.elapsedSec = 0;
      this.setState(this.status.mode === "off" ? "idle" : "armed");
      if (this.status.state === "armed") this.scheduleFakeDetection();
      this.emitStatus();
    }, 1_600);

    this.bus.emit("meeting://updated", { ...meeting });
    return { ...meeting };
  }

  // --------------------------------------------------------------- AI (mock)
  private summarize(transcript: TranscriptSegment[], title: string): MeetingSummary {
    const sentences = transcript.map((s) => s.text);
    const tldr =
      sentences.length > 0
        ? `Discussion covering ${title.toLowerCase()}. ` +
          sentences.slice(0, 2).join(" ")
        : "No speech was captured for this session.";
    return {
      tldr,
      keyPoints: sentences.slice(0, 4).map((s) => s.replace(/\.$/, "")),
      decisions: sentences
        .filter((s) => /decid|agree|commit|let's|we'll/i.test(s))
        .slice(0, 3)
        .map((s) => s.replace(/\.$/, "")),
      generatedAt: new Date().toISOString(),
      model: "local · summarizer-v1",
    };
  }

  private extractActionItems(transcript: TranscriptSegment[]): ActionItem[] {
    return transcript
      .filter((s) => /\bI'll\b|\bwe'll\b|follow up|will\b|need to|action/i.test(s.text))
      .slice(0, 5)
      .map((s) => ({
        id: nextId("a"),
        text: s.text.replace(/^.*?(I'll|we'll|need to)\s*/i, (_m, g) => `${g} `).trim(),
        done: false,
        assignee: s.speaker,
        dueDate: null,
      }));
  }

  // ---------------------------------------------------------------- helpers
  private setState(state: RecorderState) {
    this.status.state = state;
  }
  private emitStatus() {
    this.bus.emit("recorder://status", { ...this.status });
  }
  private requireMeeting(id: string): Meeting {
    const m = this.meetings.get(id);
    if (!m) throw new Error(`[mock] meeting not found: ${id}`);
    return m;
  }
}

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

const SAMPLE_DETECTIONS: Array<{
  platform: Meeting["platform"];
  title: string;
  processName: string;
}> = [
  { platform: "googleMeet", title: "Google Meet — Weekly Sync", processName: "chrome.exe" },
  { platform: "zoom", title: "Zoom Meeting — Design Review", processName: "Zoom.exe" },
  { platform: "teams", title: "Microsoft Teams — 1:1 with Priya", processName: "ms-teams.exe" },
  { platform: "slack", title: "Slack Huddle — #eng-oncall", processName: "slack.exe" },
  { platform: "discord", title: "Discord — Community Standup", processName: "Discord.exe" },
];

const LIVE_LINES: Array<{ speaker: string; text: string }> = [
  { speaker: "You", text: "Okay, let's get started — I think everyone's here now." },
  { speaker: "Priya Nair", text: "Great. First item is the release timeline for next week." },
  { speaker: "Marcus Lee", text: "I'll have the build ready by Wednesday so QA has two full days." },
  { speaker: "You", text: "Perfect. We'll need sign-off from design before we ship." },
  { speaker: "Dana Whitfield", text: "Design is good to go, I'll post the final specs in the channel." },
  { speaker: "Priya Nair", text: "Let's decide on the rollout — staged or all at once?" },
  { speaker: "You", text: "Staged. Ten percent first, then ramp if metrics look healthy." },
  { speaker: "Marcus Lee", text: "Agreed. I'll set up the feature flag for the staged rollout." },
];

export const mockBackend = new MockBackend();

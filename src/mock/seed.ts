import type {
  ActionItem,
  Meeting,
  Participant,
  TimelineMarker,
  TranscriptSegment,
} from "@/types";

let idCounter = 1000;
export const nextId = (prefix = "id") => `${prefix}_${(idCounter++).toString(36)}`;

const iso = (offsetMs: number) => new Date(Date.now() + offsetMs).toISOString();

function seg(
  speaker: string,
  text: string,
  startMs: number,
  endMs: number,
  confidence = 0.95,
): TranscriptSegment {
  return { id: nextId("seg"), speaker, text, startMs, endMs, confidence };
}

/** A fully-populated flagship meeting used to show off the detail page. */
function roadmapMeeting(): Meeting {
  const startOffset = -1000 * 60 * 60 * 4; // 4h ago
  const durationSec = 42 * 60 + 18;

  const participants: Participant[] = [
    { id: nextId("p"), name: "Alex Rivera", talkRatio: 0.34 },
    { id: nextId("p"), name: "Priya Nair", talkRatio: 0.28 },
    { id: nextId("p"), name: "Marcus Lee", talkRatio: 0.22 },
    { id: nextId("p"), name: "Dana Whitfield", talkRatio: 0.16 },
  ];

  const transcript: TranscriptSegment[] = [
    seg("Alex Rivera", "Alright, thanks everyone for hopping on. Today's really about locking the Q3 roadmap so engineering can start sprint planning Monday.", 2_000, 11_500),
    seg("Priya Nair", "Sounds good. Before we dive in — are we still treating the offline mode as a hard commitment or a stretch goal?", 12_000, 19_800),
    seg("Alex Rivera", "Hard commitment. Support keeps flagging it as the number one churn reason for field teams.", 20_100, 27_400),
    seg("Marcus Lee", "From an engineering side offline is doable in Q3, but it forces the local sync layer to land first. That's roughly three weeks.", 28_000, 38_600),
    seg("Dana Whitfield", "Design already has the conflict-resolution flows drafted, so we're not blocked there.", 39_000, 45_200),
    seg("Priya Nair", "Okay. Then I'd propose: sync layer weeks one to three, offline UI weeks three to six, and we hold the last two weeks for hardening.", 46_000, 58_900),
    seg("Alex Rivera", "I like that. Let's make hardening non-negotiable this time — we cut it last quarter and paid for it.", 59_200, 67_000),
    seg("Marcus Lee", "Agreed. I'll also want a feature flag so we can dark-launch offline to internal users first.", 67_500, 74_800),
    seg("Dana Whitfield", "One open question: do we localize the new onboarding in Q3 or push to Q4?", 75_200, 82_100),
    seg("Alex Rivera", "Push localization to Q4. Let's not overload the quarter. Decision made.", 82_500, 89_000),
    seg("Priya Nair", "Perfect. I'll write up the roadmap doc and share it by end of day tomorrow.", 89_400, 95_600),
    seg("Marcus Lee", "And I'll spike the sync layer this week so our week-one estimate is grounded.", 96_000, 103_200),
  ];

  const timeline: TimelineMarker[] = [
    { id: nextId("t"), label: "Meeting started", atMs: 0, kind: "join" },
    { id: nextId("t"), label: "Offline mode: commitment", atMs: 20_000, kind: "chapter" },
    { id: nextId("t"), label: "Proposed schedule", atMs: 46_000, kind: "highlight" },
    { id: nextId("t"), label: "Decision: localization → Q4", atMs: 82_500, kind: "action" },
    { id: nextId("t"), label: "Wrap-up", atMs: 96_000, kind: "chapter" },
  ];

  const actionItems: ActionItem[] = [
    { id: nextId("a"), text: "Write up the Q3 roadmap doc and circulate", done: false, assignee: "Priya Nair", dueDate: iso(1000 * 60 * 60 * 24) },
    { id: nextId("a"), text: "Spike the local sync layer to ground week-one estimate", done: false, assignee: "Marcus Lee", dueDate: iso(1000 * 60 * 60 * 24 * 3) },
    { id: nextId("a"), text: "Add feature flag for internal offline dark-launch", done: false, assignee: "Marcus Lee", dueDate: null },
    { id: nextId("a"), text: "Finalize conflict-resolution flows for offline UI", done: true, assignee: "Dana Whitfield", dueDate: null },
  ];

  return {
    id: "mtg_roadmap",
    title: "Q3 Product Roadmap Sync",
    platform: "googleMeet",
    mode: "record",
    status: "ready",
    startedAt: iso(startOffset),
    endedAt: iso(startOffset + durationSec * 1000),
    durationSec,
    hasAudio: true,
    hasVideo: false,
    isLocked: false,
    isStarred: true,
    isBookmarked: true,
    tags: ["roadmap", "product", "q3"],
    participants,
    timeline,
    transcript,
    actionItems,
    audioPath: "C:/Users/you/MeetApp/recordings/q3-roadmap-sync.wav",
    videoPath: null,
    summary: {
      tldr:
        "The team committed to shipping offline mode in Q3, driven by field-team churn. The plan sequences a local sync layer (weeks 1–3), offline UI (weeks 3–6), and a protected two-week hardening window. Onboarding localization was explicitly deferred to Q4 to keep the quarter focused.",
      keyPoints: [
        "Offline mode is a hard Q3 commitment, not a stretch goal.",
        "Sync layer must land first (~3 weeks) before offline UI work.",
        "Two-week hardening window is non-negotiable after last quarter's regressions.",
        "Offline will dark-launch to internal users behind a feature flag.",
      ],
      decisions: [
        "Ship offline mode in Q3.",
        "Defer onboarding localization to Q4.",
        "Reserve the final two weeks for hardening.",
      ],
      generatedAt: iso(startOffset + durationSec * 1000 + 60_000),
      model: "local · summarizer-v1",
    },
  };
}

function designCritique(): Meeting {
  const startOffset = -1000 * 60 * 60 * 26; // yesterday
  const durationSec = 28 * 60 + 5;
  return {
    id: "mtg_design",
    title: "Design Critique — Mobile Onboarding",
    platform: "zoom",
    mode: "recordVideo",
    status: "ready",
    startedAt: iso(startOffset),
    endedAt: iso(startOffset + durationSec * 1000),
    durationSec,
    hasAudio: true,
    hasVideo: true,
    isLocked: false,
    isStarred: false,
    isBookmarked: false,
    tags: ["design", "mobile"],
    participants: [
      { id: nextId("p"), name: "Dana Whitfield", talkRatio: 0.42 },
      { id: nextId("p"), name: "Sam Okafor", talkRatio: 0.33 },
      { id: nextId("p"), name: "Priya Nair", talkRatio: 0.25 },
    ],
    timeline: [
      { id: nextId("t"), label: "Meeting started", atMs: 0, kind: "join" },
      { id: nextId("t"), label: "Walkthrough of v3 flow", atMs: 30_000, kind: "chapter" },
      { id: nextId("t"), label: "Highlight: skip-to-value idea", atMs: 120_000, kind: "highlight" },
    ],
    transcript: [
      seg("Dana Whitfield", "I'll share my screen — this is the third pass on the mobile onboarding flow.", 1_500, 8_000),
      seg("Sam Okafor", "The progress dots up top read much better now. Nice.", 8_500, 13_000),
      seg("Priya Nair", "Can we let power users skip straight to the workspace and fill profile later?", 13_500, 21_000),
      seg("Dana Whitfield", "Good call — a 'skip to value' path. I'll prototype it.", 21_500, 27_000),
    ],
    actionItems: [
      { id: nextId("a"), text: "Prototype a 'skip to value' onboarding path", done: false, assignee: "Dana Whitfield", dueDate: null },
      { id: nextId("a"), text: "Run the v3 flow past 3 field users", done: false, assignee: "Sam Okafor", dueDate: null },
    ],
    audioPath: "C:/Users/you/MeetApp/recordings/design-critique.wav",
    videoPath: "C:/Users/you/MeetApp/recordings/design-critique.mp4",
    summary: {
      tldr:
        "Third iteration of the mobile onboarding flow reviewed. The new progress indicator landed well. The main new direction: add a 'skip to value' path so power users reach the workspace immediately and complete their profile later.",
      keyPoints: [
        "Progress-dot redesign improves perceived clarity.",
        "Add an optional 'skip to value' fast path for returning/power users.",
      ],
      decisions: ["Prototype the skip-to-value path before next critique."],
      generatedAt: iso(startOffset + durationSec * 1000 + 60_000),
      model: "local · summarizer-v1",
    },
  };
}

function standup(): Meeting {
  const startOffset = -1000 * 60 * 60 * 6;
  const durationSec = 15 * 60 + 40;
  return {
    id: "mtg_standup",
    title: "Weekly Engineering Standup",
    platform: "teams",
    mode: "transcribe",
    status: "ready",
    startedAt: iso(startOffset),
    endedAt: iso(startOffset + durationSec * 1000),
    durationSec,
    hasAudio: false,
    hasVideo: false,
    isLocked: true,
    isStarred: false,
    isBookmarked: false,
    tags: ["engineering", "standup"],
    participants: [
      { id: nextId("p"), name: "Marcus Lee", talkRatio: 0.3 },
      { id: nextId("p"), name: "Jules Tan", talkRatio: 0.4 },
      { id: nextId("p"), name: "Alex Rivera", talkRatio: 0.3 },
    ],
    timeline: [{ id: nextId("t"), label: "Meeting started", atMs: 0, kind: "join" }],
    transcript: [
      seg("Marcus Lee", "Quick one today. Sync layer spike is on track, PR up tomorrow.", 1_000, 7_000),
      seg("Jules Tan", "I'm unblocked on the notifications refactor now that the API shipped.", 7_500, 14_000),
      seg("Alex Rivera", "Great. No blockers from me — I'm in roadmap mode this week.", 14_500, 19_000),
    ],
    actionItems: [
      { id: nextId("a"), text: "Open sync-layer spike PR", done: false, assignee: "Marcus Lee", dueDate: null },
    ],
    audioPath: null,
    videoPath: null,
    summary: {
      tldr:
        "Short standup. Sync-layer spike is on track with a PR expected tomorrow. Notifications refactor is unblocked after the API shipped. No blockers raised.",
      keyPoints: ["Sync-layer spike on track", "Notifications refactor unblocked"],
      decisions: [],
      generatedAt: iso(startOffset + durationSec * 1000 + 30_000),
      model: "local · summarizer-v1",
    },
  };
}

/** Short clips matching the reference screenshots ("meeting-clip2", 20s). */
function clip(id: string, startOffsetMs: number, locked: boolean): Meeting {
  return {
    id,
    title: "meeting-clip2",
    platform: "unknown",
    mode: "record",
    status: "ready",
    startedAt: iso(startOffsetMs),
    endedAt: iso(startOffsetMs + 20_000),
    durationSec: 20,
    hasAudio: true,
    hasVideo: false,
    isLocked: locked,
    isStarred: false,
    isBookmarked: false,
    tags: [],
    participants: [{ id: nextId("p"), name: "You", talkRatio: 1 }],
    timeline: [{ id: nextId("t"), label: "Clip start", atMs: 0, kind: "join" }],
    transcript: [
      seg("You", "Quick note to self — remember to follow up on the vendor contract before Friday.", 500, 8_000),
      seg("You", "Also loop in finance on the renewal numbers.", 8_500, 14_000),
    ],
    actionItems: [
      { id: nextId("a"), text: "Follow up on vendor contract before Friday", done: false, assignee: "You", dueDate: null },
      { id: nextId("a"), text: "Loop in finance on renewal numbers", done: false, assignee: "You", dueDate: null },
    ],
    audioPath: "C:/Users/you/MeetApp/recordings/meeting-clip2.wav",
    videoPath: null,
    summary: {
      tldr: "A short voice memo: follow up on the vendor contract before Friday and loop finance in on the renewal numbers.",
      keyPoints: ["Vendor contract follow-up due Friday", "Finance to review renewal numbers"],
      decisions: [],
      generatedAt: iso(startOffsetMs + 25_000),
      model: "local · summarizer-v1",
    },
  };
}

export function createSeedMeetings(): Meeting[] {
  const dayAgo = -1000 * 60 * 60 * 24;
  return [
    roadmapMeeting(),
    designCritique(),
    standup(),
    clip("mtg_clip_a", dayAgo * 14, false),
    clip("mtg_clip_b", dayAgo * 14 - 60_000, true),
  ];
}

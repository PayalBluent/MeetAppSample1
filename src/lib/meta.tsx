import {
  Circle,
  FileText,
  Hash,
  MessageCircle,
  Mic,
  Power,
  Users,
  Video,
  type LucideIcon,
} from "lucide-react";
import type { CaptureMode, MeetingPlatform } from "@/types";

export interface PlatformMeta {
  label: string;
  icon: LucideIcon;
  /** Tailwind text color class for the icon/accent. */
  color: string;
  /** Tailwind background tint used behind the icon. */
  tint: string;
}

export const PLATFORM_META: Record<MeetingPlatform, PlatformMeta> = {
  googleMeet: {
    label: "Google Meet",
    icon: Video,
    color: "text-emerald-500",
    tint: "bg-emerald-500/10",
  },
  zoom: {
    label: "Zoom",
    icon: Video,
    color: "text-blue-500",
    tint: "bg-blue-500/10",
  },
  teams: {
    label: "Microsoft Teams",
    icon: Users,
    color: "text-indigo-500",
    tint: "bg-indigo-500/10",
  },
  discord: {
    label: "Discord",
    icon: MessageCircle,
    color: "text-violet-500",
    tint: "bg-violet-500/10",
  },
  slack: {
    label: "Slack Huddle",
    icon: Hash,
    color: "text-pink-500",
    tint: "bg-pink-500/10",
  },
  webex: {
    label: "Webex",
    icon: Video,
    color: "text-teal-500",
    tint: "bg-teal-500/10",
  },
  unknown: {
    label: "Recording",
    icon: Mic,
    color: "text-muted-foreground",
    tint: "bg-muted",
  },
};

export interface ModeMeta {
  label: string;
  short: string;
  icon: LucideIcon;
  description: string;
}

export const MODE_META: Record<CaptureMode, ModeMeta> = {
  off: {
    label: "Off",
    short: "Off",
    icon: Power,
    description: "The note taker is idle.",
  },
  transcribe: {
    label: "Transcribe",
    short: "Transcribe",
    icon: FileText,
    description: "Live transcript only — no audio is saved.",
  },
  record: {
    label: "Record",
    short: "Record",
    icon: Circle,
    description: "Save microphone + system audio, with transcript.",
  },
  recordVideo: {
    label: "Record Video",
    short: "Video",
    icon: Video,
    description: "Record screen + audio and generate a transcript.",
  },
};

export const ORDERED_MODES: CaptureMode[] = [
  "transcribe",
  "record",
  "recordVideo",
  "off",
];

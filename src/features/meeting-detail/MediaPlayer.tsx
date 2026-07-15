import {
  Download,
  FileAudio,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  Video as VideoIcon,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { QuickTip } from "@/components/ui/tooltip";
import { cn, formatTimestamp } from "@/lib/utils";
import { Waveform } from "./Waveform";
import type { Playback } from "./usePlayback";
import type { Meeting } from "@/types";

export function MediaPlayer({
  meeting,
  playback,
  audioSrc,
  videoSrc,
  onMediaEl,
  onOpenFile,
}: {
  meeting: Meeting;
  playback: Playback;
  audioSrc?: string;
  videoSrc?: string;
  onMediaEl: (el: HTMLMediaElement | null) => void;
  onOpenFile: () => void;
}) {
  const {
    currentMs,
    durationMs,
    playing,
    toggle,
    seek,
    skip,
    rate,
    cycleRate,
    volume,
    muted,
    setVolume,
    toggleMute,
  } = playback;
  const progress = durationMs ? currentMs / durationMs : 0;
  const effectiveVolume = muted ? 0 : volume;
  const VolumeIcon =
    effectiveVolume === 0 ? VolumeX : effectiveVolume < 0.5 ? Volume1 : Volume2;

  return (
    <div className="overflow-hidden rounded-xl border border-border bg-card">
      {/* Video surface (real element if available, else a branded placeholder) */}
      {meeting.hasVideo ? (
        videoSrc ? (
          <video
            ref={onMediaEl}
            src={videoSrc}
            className="aspect-video w-full bg-black"
            onClick={toggle}
          />
        ) : (
          <button
            onClick={toggle}
            className="group relative flex aspect-video w-full items-center justify-center bg-gradient-to-br from-slate-900 to-slate-800"
          >
            <div className="absolute inset-0 opacity-20 [background:radial-gradient(circle_at_30%_30%,white,transparent_60%)]" />
            <span className="grid size-16 place-items-center rounded-full bg-white/10 backdrop-blur transition-transform group-hover:scale-105">
              {playing ? (
                <Pause className="size-7 text-white" />
              ) : (
                <Play className="size-7 translate-x-0.5 text-white" />
              )}
            </span>
            <span className="absolute bottom-3 left-3 flex items-center gap-1.5 rounded-md bg-black/40 px-2 py-1 text-xs text-white/90">
              <VideoIcon className="size-3.5" />
              Screen recording
            </span>
          </button>
        )
      ) : (
        <div className="flex items-center gap-3 border-b border-border bg-secondary/30 px-4 py-3">
          <span className="grid size-9 place-items-center rounded-lg bg-primary/10 text-primary">
            <FileAudio className="size-4" />
          </span>
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">Audio recording</p>
            <p className="text-xs text-muted-foreground">
              {meeting.audioPath
                ? meeting.audioPath.split("/").pop()
                : "In-memory session"}
            </p>
          </div>
        </div>
      )}

      {/* Hidden audio element for real audio-only playback */}
      {!meeting.hasVideo && audioSrc && (
        <audio ref={onMediaEl} src={audioSrc} className="hidden" />
      )}

      {/* Transport */}
      <div className="space-y-3 p-4">
        <Waveform
          seed={meeting.id}
          progress={progress}
          onSeek={(f) => seek(f * durationMs)}
        />

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1">
            <QuickTip label="Back 10s">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => skip(-10_000)}
              >
                <RotateCcw className="size-4" />
              </Button>
            </QuickTip>
            <Button
              variant="brand"
              size="icon"
              className="rounded-full"
              onClick={toggle}
            >
              {playing ? (
                <Pause className="size-5" />
              ) : (
                <Play className="size-5 translate-x-0.5" />
              )}
            </Button>
            <QuickTip label="Forward 10s">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => skip(10_000)}
              >
                <RotateCw className="size-4" />
              </Button>
            </QuickTip>
          </div>

          <div className="flex items-center gap-2">
            <span className="font-mono text-xs tabular-nums text-muted-foreground">
              {formatTimestamp(currentMs)} / {formatTimestamp(durationMs)}
            </span>
            <QuickTip label="Playback speed">
              <button
                onClick={cycleRate}
                className={cn(
                  "no-drag h-7 rounded-md border border-border px-2 text-xs font-medium tabular-nums hover:bg-secondary",
                )}
              >
                {rate}×
              </button>
            </QuickTip>
            <QuickTip label="Open file">
              <Button variant="ghost" size="icon-sm" onClick={onOpenFile}>
                <Download className="size-4" />
              </Button>
            </QuickTip>
          </div>
        </div>

        {/* Volume — mute toggle + slider. Own row so it stays visible in the
            narrow detail sidebar. */}
        <div className="flex items-center gap-2 pt-0.5">
          <QuickTip label={muted ? "Unmute" : "Mute"}>
            <Button variant="ghost" size="icon-sm" onClick={toggleMute}>
              <VolumeIcon className="size-4" />
            </Button>
          </QuickTip>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={effectiveVolume}
            onChange={(e) => setVolume(parseFloat(e.target.value))}
            aria-label="Playback volume"
            className="h-1.5 flex-1 cursor-pointer"
            style={{ accentColor: "hsl(var(--primary))" }}
          />
          <span className="w-9 text-right font-mono text-xs tabular-nums text-muted-foreground">
            {Math.round(effectiveVolume * 100)}%
          </span>
        </div>
      </div>
    </div>
  );
}

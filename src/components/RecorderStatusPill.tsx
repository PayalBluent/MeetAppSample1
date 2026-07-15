import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { formatTimestamp } from "@/lib/utils";
import { useRecorderStatus } from "@/hooks/useMeetings";
import { MODE_META } from "@/lib/meta";
import type { RecorderState } from "@/types";

const STATE_LABEL: Record<RecorderState, string> = {
  idle: "Idle",
  armed: "Armed",
  detecting: "Meeting detected",
  recording: "Recording",
  processing: "Processing",
  error: "Error",
};

export function RecorderStatusPill({ className }: { className?: string }) {
  const { data: status } = useRecorderStatus();
  if (!status) return null;

  const recording = status.state === "recording";
  // Recording but audio hasn't started flowing yet (devices still opening).
  const starting = recording && !status.audioReady;
  const live = recording && status.audioReady;
  const active = recording || status.state === "detecting";
  const mode = MODE_META[status.mode];

  return (
    <div
      className={cn(
        "no-drag flex items-center gap-2 rounded-full border border-border bg-secondary/60 px-3 py-1 text-xs font-medium",
        className,
      )}
    >
      <span className="relative flex size-2.5 items-center justify-center">
        <span
          className={cn(
            "size-2 rounded-full",
            starting && "bg-muted-foreground/60 animate-pulse",
            live && "bg-destructive animate-rec-pulse",
            status.state === "detecting" && "bg-warning",
            status.state === "processing" && "bg-primary",
            status.state === "armed" && "bg-primary/70",
            status.state === "idle" && "bg-muted-foreground/50",
            status.state === "error" && "bg-destructive",
          )}
        />
      </span>

      <span className="text-foreground">
        {starting ? "Getting audio…" : STATE_LABEL[status.state]}
      </span>

      {active && (
        <>
          <span className="text-muted-foreground">·</span>
          <span className="text-muted-foreground">{mode.short}</span>
        </>
      )}

      {live && (
        <motion.span
          key="elapsed"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="tabular-nums text-foreground"
        >
          {formatTimestamp(status.elapsedSec * 1000)}
        </motion.span>
      )}
    </div>
  );
}

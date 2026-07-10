import { motion } from "motion/react";
import { Radio, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PLATFORM_META } from "@/lib/meta";
import type { DetectedMeeting } from "@/types";

export function DetectionCard({
  detection,
  onCapture,
  onDismiss,
  busy,
}: {
  detection: DetectedMeeting;
  onCapture: () => void;
  onDismiss: () => void;
  busy?: boolean;
}) {
  const meta = PLATFORM_META[detection.platform];
  return (
    <motion.div
      initial={{ opacity: 0, y: -8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -8, scale: 0.98 }}
      className="rounded-xl border border-primary/30 bg-accent/60 p-3"
    >
      <div className="flex items-center gap-2.5">
        <span className={`grid size-9 place-items-center rounded-lg ${meta.tint}`}>
          <meta.icon className={`size-4.5 ${meta.color}`} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <Radio className="size-3 shrink-0 text-primary" />
            <span className="truncate text-[11px] font-semibold uppercase tracking-wide text-primary">
              Meeting detected
            </span>
          </div>
          <p className="truncate text-sm font-medium text-foreground">
            {meta.label}
          </p>
        </div>
        <button
          onClick={onDismiss}
          aria-label="Dismiss"
          className="no-drag grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-secondary hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="mt-3 flex gap-2">
        <Button
          size="sm"
          variant="brand"
          className="flex-1"
          onClick={onCapture}
          disabled={busy}
        >
          Start capture
        </Button>
        <Button size="sm" variant="ghost" onClick={onDismiss}>
          Ignore
        </Button>
      </div>
    </motion.div>
  );
}

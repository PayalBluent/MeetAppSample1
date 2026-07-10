import {
  Flag,
  LogIn,
  LogOut,
  Sparkles,
  Star,
  type LucideIcon,
} from "lucide-react";
import { cn, formatTimestamp } from "@/lib/utils";
import type { TimelineMarker } from "@/types";

const KIND_ICON: Record<TimelineMarker["kind"], LucideIcon> = {
  chapter: Flag,
  highlight: Star,
  action: Sparkles,
  join: LogIn,
  leave: LogOut,
};

const KIND_COLOR: Record<TimelineMarker["kind"], string> = {
  chapter: "text-primary bg-primary/10",
  highlight: "text-amber-500 bg-amber-500/10",
  action: "text-violet-500 bg-violet-500/10",
  join: "text-emerald-500 bg-emerald-500/10",
  leave: "text-rose-500 bg-rose-500/10",
};

export function TimelineView({
  markers,
  currentMs,
  onSeek,
}: {
  markers: TimelineMarker[];
  currentMs: number;
  onSeek: (ms: number) => void;
}) {
  if (markers.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        No timeline markers.
      </p>
    );
  }

  return (
    <ol className="relative space-y-1 pl-2">
      <span className="absolute bottom-3 left-[19px] top-3 w-px bg-border" />
      {markers.map((m) => {
        const Icon = KIND_ICON[m.kind];
        const active = currentMs >= m.atMs;
        return (
          <li key={m.id}>
            <button
              onClick={() => onSeek(m.atMs)}
              className="no-drag group relative flex w-full items-center gap-3 rounded-lg px-1 py-1.5 text-left hover:bg-secondary/60"
            >
              <span
                className={cn(
                  "relative z-10 grid size-8 shrink-0 place-items-center rounded-full ring-4 ring-background",
                  KIND_COLOR[m.kind],
                  !active && "opacity-60",
                )}
              >
                <Icon className="size-4" />
              </span>
              <span className="min-w-0 flex-1 truncate text-sm text-foreground/90 group-hover:text-foreground">
                {m.label}
              </span>
              <span className="font-mono text-xs tabular-nums text-muted-foreground">
                {formatTimestamp(m.atMs)}
              </span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}

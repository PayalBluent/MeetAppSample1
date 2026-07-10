import { useMemo, useRef } from "react";
import { cn, hashString } from "@/lib/utils";

/**
 * Deterministic pseudo-waveform derived from the meeting id. Click / drag to
 * seek. Bars before the playhead render in the accent colour.
 */
export function Waveform({
  seed,
  progress,
  onSeek,
  className,
  bars = 96,
}: {
  seed: string;
  /** 0..1 */
  progress: number;
  onSeek: (fraction: number) => void;
  className?: string;
  bars?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);

  const heights = useMemo(() => {
    const base = hashString(seed);
    return Array.from({ length: bars }, (_, i) => {
      // Smooth-ish pseudo-random envelope.
      const n =
        Math.sin(i * 0.35 + base) * 0.5 +
        Math.sin(i * 0.11 + base * 0.7) * 0.3 +
        Math.sin(i * 0.9 + base * 1.3) * 0.2;
      return 0.2 + Math.abs(n) * 0.8;
    });
  }, [seed, bars]);

  const seekFromEvent = (clientX: number) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const fraction = (clientX - rect.left) / rect.width;
    onSeek(Math.max(0, Math.min(1, fraction)));
  };

  return (
    <div
      ref={ref}
      role="slider"
      aria-label="Seek"
      aria-valuenow={Math.round(progress * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
      tabIndex={0}
      onPointerDown={(e) => {
        (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
        seekFromEvent(e.clientX);
      }}
      onPointerMove={(e) => {
        if (e.buttons === 1) seekFromEvent(e.clientX);
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") onSeek(Math.min(1, progress + 0.02));
        if (e.key === "ArrowLeft") onSeek(Math.max(0, progress - 0.02));
      }}
      className={cn(
        "flex h-14 cursor-pointer items-center gap-[2px] focus:outline-none",
        className,
      )}
    >
      {heights.map((h, i) => {
        const played = i / bars <= progress;
        return (
          <span
            key={i}
            style={{ height: `${h * 100}%` }}
            className={cn(
              "min-h-[2px] flex-1 rounded-full transition-colors",
              played ? "bg-primary" : "bg-muted-foreground/25",
            )}
          />
        );
      })}
    </div>
  );
}

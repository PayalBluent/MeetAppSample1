import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { MODE_META, ORDERED_MODES } from "@/lib/meta";
import type { CaptureMode } from "@/types";

/**
 * Segmented control for the four capture modes, matching the reference panel:
 * Transcribe · Record · Record Video · Off (Off renders in destructive red).
 */
export function ModeSelector({
  value,
  onChange,
  disabled,
}: {
  value: CaptureMode;
  onChange: (mode: CaptureMode) => void;
  disabled?: boolean;
}) {
  return (
    <div
      role="tablist"
      aria-label="Capture mode"
      className="grid grid-cols-4 gap-1.5 rounded-xl border border-border bg-secondary/50 p-1.5"
    >
      {ORDERED_MODES.map((mode) => {
        const meta = MODE_META[mode];
        const active = value === mode;
        const isOff = mode === "off";
        return (
          <button
            key={mode}
            role="tab"
            aria-selected={active}
            disabled={disabled}
            onClick={() => onChange(mode)}
            title={meta.description}
            className={cn(
              "no-drag relative flex h-14 flex-col items-center justify-center gap-1 rounded-lg text-[11px] font-medium transition-colors disabled:opacity-50",
              active
                ? isOff
                  ? "text-destructive-foreground"
                  : "text-primary-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {active && (
              <motion.span
                layoutId="mode-active"
                className={cn(
                  "absolute inset-0 rounded-lg shadow-sm",
                  isOff ? "bg-destructive" : "bg-brand",
                )}
                transition={{ type: "spring", stiffness: 500, damping: 34 }}
              />
            )}
            <meta.icon
              className={cn("relative size-4", isOff && active && "fill-current/0")}
            />
            <span className="relative leading-none">{meta.short}</span>
          </button>
        );
      })}
    </div>
  );
}

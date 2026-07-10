import { cn } from "@/lib/utils";

const BARS = 14;

/** Segmented level meter. `level` is 0..1. */
export function AudioMeter({
  level,
  className,
  tone = "primary",
}: {
  level: number;
  className?: string;
  tone?: "primary" | "destructive";
}) {
  const active = Math.round(level * BARS);
  return (
    <div className={cn("flex h-4 items-end gap-[3px]", className)}>
      {Array.from({ length: BARS }).map((_, i) => {
        const on = i < active;
        const height = 30 + (i / BARS) * 70; // rising profile
        return (
          <span
            key={i}
            style={{ height: `${height}%` }}
            className={cn(
              "w-[3px] rounded-full transition-all duration-100",
              on
                ? tone === "destructive"
                  ? "bg-destructive"
                  : "bg-primary"
                : "bg-muted",
            )}
          />
        );
      })}
    </div>
  );
}

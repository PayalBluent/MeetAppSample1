import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { hashString, initials } from "@/lib/utils";
import type { Participant } from "@/types";

const RING = [
  "bg-violet-500/15 text-violet-500",
  "bg-sky-500/15 text-sky-500",
  "bg-emerald-500/15 text-emerald-500",
  "bg-amber-500/15 text-amber-500",
  "bg-rose-500/15 text-rose-500",
  "bg-teal-500/15 text-teal-500",
];

export function Participants({ people }: { people: Participant[] }) {
  if (people.length === 0) return null;
  return (
    <div className="space-y-2.5">
      {people.map((p) => {
        const tone = RING[hashString(p.name) % RING.length]!;
        const ratio = Math.round((p.talkRatio ?? 0) * 100);
        return (
          <div key={p.id} className="flex items-center gap-3">
            <Avatar className="size-8">
              <AvatarFallback className={tone}>
                {initials(p.name)}
              </AvatarFallback>
            </Avatar>
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between">
                <span className="truncate text-sm font-medium">{p.name}</span>
                {p.talkRatio != null && (
                  <span className="text-xs text-muted-foreground">{ratio}%</span>
                )}
              </div>
              {p.talkRatio != null && (
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full bg-primary/70"
                    style={{ width: `${ratio}%` }}
                  />
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

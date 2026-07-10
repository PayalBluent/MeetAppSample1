import { CalendarClock, ListChecks, UserRound } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { cn, formatCalendarDate } from "@/lib/utils";
import type { Meeting } from "@/types";

export function ActionItems({
  meeting,
  onToggle,
}: {
  meeting: Meeting;
  onToggle: (itemId: string, done: boolean) => void;
}) {
  const items = meeting.actionItems;
  const done = items.filter((i) => i.done).length;

  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border py-10 text-center">
        <ListChecks className="size-6 text-muted-foreground" />
        <p className="text-sm text-muted-foreground">
          No action items detected.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <ListChecks className="size-4 text-muted-foreground" />
          Action items
        </h3>
        <span className="text-xs text-muted-foreground">
          {done}/{items.length} done
        </span>
      </div>

      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-brand transition-all"
          style={{ width: `${items.length ? (done / items.length) * 100 : 0}%` }}
        />
      </div>

      <ul className="space-y-1.5">
        {items.map((item) => (
          <li
            key={item.id}
            className="flex items-start gap-3 rounded-lg border border-border bg-card px-3 py-2.5"
          >
            <div className="pt-0.5">
              <Checkbox
                checked={item.done}
                onCheckedChange={(v) => onToggle(item.id, v)}
              />
            </div>
            <div className="min-w-0 flex-1">
              <p
                className={cn(
                  "selectable text-sm leading-snug",
                  item.done && "text-muted-foreground line-through",
                )}
              >
                {item.text}
              </p>
              <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                {item.assignee && (
                  <span className="flex items-center gap-1">
                    <UserRound className="size-3" />
                    {item.assignee}
                  </span>
                )}
                {item.dueDate && (
                  <span className="flex items-center gap-1">
                    <CalendarClock className="size-3" />
                    {formatCalendarDate(item.dueDate)}
                  </span>
                )}
              </div>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

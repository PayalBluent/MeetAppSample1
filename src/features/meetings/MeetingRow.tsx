import { useNavigate } from "react-router-dom";
import {
  Bookmark,
  Lock,
  LockOpen,
  MoreVertical,
  Pencil,
  Star,
  Tag,
  Trash2,
  Video,
} from "lucide-react";
import { cn, formatCalendarDate, formatDuration } from "@/lib/utils";
import { PLATFORM_META } from "@/lib/meta";
import { Checkbox } from "@/components/ui/checkbox";
import { QuickTip } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Meeting } from "@/types";

interface MeetingRowProps {
  meeting: Meeting;
  selected: boolean;
  anySelected: boolean;
  onToggleSelect: () => void;
  onToggleFlag: (flag: "locked" | "starred" | "bookmarked") => void;
  onDelete: () => void;
  onRename: () => void;
}

export function MeetingRow({
  meeting,
  selected,
  anySelected,
  onToggleSelect,
  onToggleFlag,
  onDelete,
  onRename,
}: MeetingRowProps) {
  const navigate = useNavigate();
  const meta = PLATFORM_META[meeting.platform];
  const showControls = selected || anySelected;

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => navigate(`/app/meetings/${meeting.id}`)}
      onKeyDown={(e) => {
        if (e.key === "Enter") navigate(`/app/meetings/${meeting.id}`);
      }}
      className={cn(
        "group relative flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 transition-colors",
        selected ? "bg-secondary" : "hover:bg-secondary/60",
      )}
    >
      {/* Leading: checkbox on hover/selection, else platform icon */}
      <div className="relative size-9 shrink-0">
        <span
          className={cn(
            "absolute inset-0 grid place-items-center rounded-lg transition-opacity",
            meta.tint,
            showControls ? "opacity-0" : "opacity-100 group-hover:opacity-0",
          )}
        >
          <meta.icon className={cn("size-4", meta.color)} />
        </span>
        <div
          className={cn(
            "absolute inset-0 grid place-items-center transition-opacity",
            showControls ? "opacity-100" : "opacity-0 group-hover:opacity-100",
          )}
        >
          <Checkbox
            checked={selected}
            onCheckedChange={onToggleSelect}
            aria-label={`Select ${meeting.title}`}
          />
        </div>
      </div>

      {/* Title + meta */}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-semibold text-foreground">
            {meeting.title}
          </span>
          <span className="shrink-0 text-xs text-muted-foreground">
            {formatDuration(meeting.durationSec)}
          </span>
          {meeting.isStarred && (
            <Star className="size-3 shrink-0 fill-amber-400 text-amber-400" />
          )}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="truncate">{meta.label}</span>
          {meeting.hasVideo && (
            <>
              <span>·</span>
              <Video className="size-3" />
            </>
          )}
          {meeting.tags.length > 0 && (
            <>
              <span>·</span>
              <span className="truncate">
                {meeting.tags.map((t) => `#${t}`).join(" ")}
              </span>
            </>
          )}
        </div>
      </div>

      {/* Trailing: hover actions, else lock + date */}
      <div className="flex shrink-0 items-center">
        <div
          className={cn(
            "flex items-center gap-0.5 transition-opacity",
            "opacity-0 group-hover:opacity-100",
            selected && "opacity-100",
          )}
        >
          <RowAction label="Tags" onClick={() => onToggleFlag("bookmarked")}>
            <Tag className="size-4" />
          </RowAction>
          <RowAction
            label={meeting.isLocked ? "Unlock" : "Lock"}
            active={meeting.isLocked}
            onClick={() => onToggleFlag("locked")}
          >
            {meeting.isLocked ? (
              <Lock className="size-4" />
            ) : (
              <LockOpen className="size-4" />
            )}
          </RowAction>
          <RowAction
            label={meeting.isStarred ? "Unstar" : "Star"}
            active={meeting.isStarred}
            onClick={() => onToggleFlag("starred")}
          >
            <Star
              className={cn(
                "size-4",
                meeting.isStarred && "fill-amber-400 text-amber-400",
              )}
            />
          </RowAction>
          <RowAction
            label={meeting.isBookmarked ? "Remove bookmark" : "Bookmark"}
            active={meeting.isBookmarked}
            onClick={() => onToggleFlag("bookmarked")}
          >
            <Bookmark
              className={cn(
                "size-4",
                meeting.isBookmarked && "fill-primary text-primary",
              )}
            />
          </RowAction>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                onClick={(e) => e.stopPropagation()}
                aria-label="More"
                className="no-drag grid size-8 place-items-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground"
              >
                <MoreVertical className="size-4" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              onClick={(e) => e.stopPropagation()}
            >
              <DropdownMenuItem onSelect={onRename}>
                <Pencil />
                Rename
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem destructive onSelect={onDelete}>
                <Trash2 />
                Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {/* Resting state: lock + date (hidden on hover) */}
        <div
          className={cn(
            "flex items-center gap-2 pr-1 text-xs text-muted-foreground transition-opacity group-hover:opacity-0",
            selected && "hidden",
          )}
        >
          {meeting.isLocked && <Lock className="size-3.5" />}
          <span className="tabular-nums">
            {formatCalendarDate(meeting.startedAt)}
          </span>
        </div>
      </div>
    </div>
  );
}

function RowAction({
  children,
  label,
  onClick,
  active,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
  active?: boolean;
}) {
  return (
    <QuickTip label={label}>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onClick();
        }}
        aria-label={label}
        className={cn(
          "no-drag grid size-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground",
          active && "text-foreground",
        )}
      >
        {children}
      </button>
    </QuickTip>
  );
}

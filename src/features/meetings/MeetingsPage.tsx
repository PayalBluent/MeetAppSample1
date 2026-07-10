import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowDownUp,
  Bookmark,
  CheckCheck,
  Lock,
  Plus,
  Search,
  SlidersHorizontal,
  Sparkles,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { QuickTip } from "@/components/ui/tooltip";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { PLATFORM_META } from "@/lib/meta";
import {
  useDeleteMeeting,
  useMeetings,
  useRenameMeeting,
  useStartCapture,
  useToggleFlag,
} from "@/hooks/useMeetings";
import { useUIStore, type SortMode, type ViewFilter } from "@/stores/uiStore";
import { MeetingRow } from "./MeetingRow";
import type { Meeting } from "@/types";

const SORT_LABELS: Record<SortMode, string> = {
  recent: "Most recent",
  oldest: "Oldest first",
  longest: "Longest",
  title: "Title (A–Z)",
};

export function MeetingsPage() {
  const navigate = useNavigate();
  const { data: meetings, isLoading } = useMeetings();
  const toggleFlag = useToggleFlag();
  const deleteMeeting = useDeleteMeeting();
  const renameMeeting = useRenameMeeting();
  const startCapture = useStartCapture();

  const {
    search,
    setSearch,
    sort,
    setSort,
    filter,
    setFilter,
    selected,
    toggleSelected,
    clearSelected,
  } = useUIStore();

  const [renameTarget, setRenameTarget] = useState<Meeting | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<Meeting | null>(null);

  const visible = useMemo(
    () => filterAndSort(meetings ?? [], { search, sort, filter }),
    [meetings, search, sort, filter],
  );

  const anySelected = selected.size > 0;

  const handleNew = async () => {
    const status = await startCapture.mutateAsync({ title: "New recording" });
    if (status.activeMeetingId) {
      navigate(`/app/meetings/${status.activeMeetingId}`);
    }
  };

  const openRename = (m: Meeting) => {
    setRenameTarget(m);
    setRenameValue(m.title);
  };

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="shrink-0 px-6 pb-3 pt-5">
        <div className="flex items-center justify-between gap-4">
          <h1 className="text-2xl font-bold tracking-tight text-foreground">
            Meeting notes
          </h1>
          <div className="flex items-center gap-1">
            <FilterMenu filter={filter} setFilter={setFilter} />
            <SortMenu sort={sort} setSort={setSort} />
            <Button
              variant="brand"
              size="sm"
              className="ml-1.5 h-9 px-4"
              onClick={handleNew}
              disabled={startCapture.isPending}
            >
              <Plus />
              New
            </Button>
          </div>
        </div>

        {/* Search */}
        <div className="relative mt-4">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search meetings, tags, platforms…"
            className="h-10 pl-9"
          />
          {search && (
            <button
              onClick={() => setSearch("")}
              className="absolute right-2.5 top-1/2 grid size-6 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-secondary hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          )}
        </div>
      </div>

      {/* Selection action bar */}
      <AnimatePresence>
        {anySelected && (
          <motion.div
            initial={{ opacity: 0, y: -6, height: 0 }}
            animate={{ opacity: 1, y: 0, height: "auto" }}
            exit={{ opacity: 0, y: -6, height: 0 }}
            className="overflow-hidden px-6"
          >
            <div className="mb-2 flex items-center gap-2 rounded-lg border border-border bg-secondary/60 px-3 py-2">
              <span className="text-sm font-medium">
                {selected.size} selected
              </span>
              <div className="mx-1 h-4 w-px bg-border" />
              <BulkAction
                icon={<Star className="size-4" />}
                label="Star"
                onClick={() =>
                  selected.forEach((id) =>
                    toggleFlag.mutate({ id, flag: "starred" }),
                  )
                }
              />
              <BulkAction
                icon={<Bookmark className="size-4" />}
                label="Bookmark"
                onClick={() =>
                  selected.forEach((id) =>
                    toggleFlag.mutate({ id, flag: "bookmarked" }),
                  )
                }
              />
              <BulkAction
                icon={<Lock className="size-4" />}
                label="Lock"
                onClick={() =>
                  selected.forEach((id) =>
                    toggleFlag.mutate({ id, flag: "locked" }),
                  )
                }
              />
              <BulkAction
                icon={<Trash2 className="size-4" />}
                label="Delete"
                destructive
                onClick={() => {
                  selected.forEach((id) => deleteMeeting.mutate(id));
                  clearSelected();
                }}
              />
              <Button
                variant="ghost"
                size="sm"
                className="ml-auto"
                onClick={clearSelected}
              >
                <CheckCheck className="size-4" />
                Clear
              </Button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* List */}
      <ScrollArea className="min-h-0 flex-1">
        <div className="px-4 pb-8">
          {isLoading ? (
            <ListSkeleton />
          ) : visible.length === 0 ? (
            <EmptyState hasQuery={!!search || filter !== "all"} onNew={handleNew} />
          ) : (
            <motion.div layout className="flex flex-col">
              <AnimatePresence initial={false}>
                {visible.map((m) => (
                  <motion.div
                    key={m.id}
                    layout
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0, height: 0 }}
                  >
                    <MeetingRow
                      meeting={m}
                      selected={selected.has(m.id)}
                      anySelected={anySelected}
                      onToggleSelect={() => toggleSelected(m.id)}
                      onToggleFlag={(flag) =>
                        toggleFlag.mutate({ id: m.id, flag })
                      }
                      onRename={() => openRename(m)}
                      onDelete={() => setDeleteTarget(m)}
                    />
                    <div className="mx-3 h-px bg-border/60 last:hidden" />
                  </motion.div>
                ))}
              </AnimatePresence>
            </motion.div>
          )}
        </div>
      </ScrollArea>

      {/* Rename dialog */}
      <Dialog
        open={!!renameTarget}
        onOpenChange={(o) => !o && setRenameTarget(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rename meeting</DialogTitle>
            <DialogDescription>
              Give this meeting a clearer title.
            </DialogDescription>
          </DialogHeader>
          <Input
            autoFocus
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && renameTarget && renameValue.trim()) {
                renameMeeting.mutate({
                  id: renameTarget.id,
                  title: renameValue.trim(),
                });
                setRenameTarget(null);
              }
            }}
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRenameTarget(null)}>
              Cancel
            </Button>
            <Button
              disabled={!renameValue.trim()}
              onClick={() => {
                if (renameTarget) {
                  renameMeeting.mutate({
                    id: renameTarget.id,
                    title: renameValue.trim(),
                  });
                }
                setRenameTarget(null);
              }}
            >
              Save
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete confirm */}
      <Dialog
        open={!!deleteTarget}
        onOpenChange={(o) => !o && setDeleteTarget(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete meeting?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.title}” and its transcript will be permanently
              removed. This can’t be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                if (deleteTarget) deleteMeeting.mutate(deleteTarget.id);
                setDeleteTarget(null);
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function filterAndSort(
  meetings: Meeting[],
  opts: { search: string; sort: SortMode; filter: ViewFilter },
): Meeting[] {
  const q = opts.search.trim().toLowerCase();
  let list = meetings.filter((m) => {
    if (opts.filter === "starred" && !m.isStarred) return false;
    if (opts.filter === "bookmarked" && !m.isBookmarked) return false;
    if (opts.filter === "locked" && !m.isLocked) return false;
    if (!q) return true;
    const hay =
      `${m.title} ${m.tags.join(" ")} ${PLATFORM_META[m.platform].label}`.toLowerCase();
    return hay.includes(q);
  });

  list = [...list].sort((a, b) => {
    switch (opts.sort) {
      case "oldest":
        return +new Date(a.startedAt) - +new Date(b.startedAt);
      case "longest":
        return b.durationSec - a.durationSec;
      case "title":
        return a.title.localeCompare(b.title);
      case "recent":
      default:
        return +new Date(b.startedAt) - +new Date(a.startedAt);
    }
  });
  return list;
}

function FilterMenu({
  filter,
  setFilter,
}: {
  filter: ViewFilter;
  setFilter: (f: ViewFilter) => void;
}) {
  const filters: { key: ViewFilter; label: string }[] = [
    { key: "all", label: "All meetings" },
    { key: "starred", label: "Starred" },
    { key: "bookmarked", label: "Bookmarked" },
    { key: "locked", label: "Locked" },
  ];
  return (
    <DropdownMenu>
      <QuickTip label="Filter">
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            className={cn(filter !== "all" && "text-primary")}
          >
            <SlidersHorizontal className="size-[18px]" />
          </Button>
        </DropdownMenuTrigger>
      </QuickTip>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Show</DropdownMenuLabel>
        {filters.map((f) => (
          <DropdownMenuCheckboxItem
            key={f.key}
            checked={filter === f.key}
            onCheckedChange={() => setFilter(f.key)}
          >
            {f.label}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SortMenu({
  sort,
  setSort,
}: {
  sort: SortMode;
  setSort: (s: SortMode) => void;
}) {
  return (
    <DropdownMenu>
      <QuickTip label="Sort">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon">
            <ArrowDownUp className="size-[18px]" />
          </Button>
        </DropdownMenuTrigger>
      </QuickTip>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Sort by</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={sort}
          onValueChange={(v) => setSort(v as SortMode)}
        >
          {(Object.keys(SORT_LABELS) as SortMode[]).map((key) => (
            <DropdownMenuRadioItem key={key} value={key}>
              {SORT_LABELS[key]}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function BulkAction({
  icon,
  label,
  onClick,
  destructive,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <QuickTip label={label}>
      <button
        onClick={onClick}
        aria-label={label}
        className={cn(
          "grid size-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground",
          destructive && "hover:text-destructive",
        )}
      >
        {icon}
      </button>
    </QuickTip>
  );
}

function ListSkeleton() {
  return (
    <div className="space-y-1 pt-1">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 px-3 py-2.5">
          <Skeleton className="size-9 rounded-lg" />
          <div className="flex-1 space-y-2">
            <Skeleton className="h-3.5 w-48" />
            <Skeleton className="h-3 w-28" />
          </div>
          <Skeleton className="h-3 w-10" />
        </div>
      ))}
    </div>
  );
}

function EmptyState({
  hasQuery,
  onNew,
}: {
  hasQuery: boolean;
  onNew: () => void;
}) {
  return (
    <div className="flex flex-col items-center justify-center px-6 py-24 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-accent">
        <Sparkles className="size-6 text-primary" />
      </div>
      <h3 className="mt-4 text-base font-semibold">
        {hasQuery ? "No matching meetings" : "No meetings yet"}
      </h3>
      <p className="mt-1 max-w-xs text-sm text-muted-foreground">
        {hasQuery
          ? "Try a different search or clear your filters."
          : "Start a recording or join a call — detected meetings show up here automatically."}
      </p>
      {!hasQuery && (
        <Button variant="brand" className="mt-5" onClick={onNew}>
          <Plus />
          Start a recording
        </Button>
      )}
    </div>
  );
}

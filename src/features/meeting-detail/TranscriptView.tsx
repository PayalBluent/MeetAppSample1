import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import { Check, Copy, Loader2, ScrollText, Search, X } from "lucide-react";
import { cn, formatTimestamp, hashString, initials } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { Meeting, TranscriptSegment } from "@/types";

const SPEAKER_COLORS = [
  "text-violet-500 bg-violet-500/10",
  "text-sky-500 bg-sky-500/10",
  "text-emerald-500 bg-emerald-500/10",
  "text-amber-500 bg-amber-500/10",
  "text-rose-500 bg-rose-500/10",
  "text-teal-500 bg-teal-500/10",
];

function speakerColor(name: string) {
  return SPEAKER_COLORS[hashString(name) % SPEAKER_COLORS.length]!;
}

export function TranscriptView({
  meeting,
  currentMs,
  onSeek,
  onTranscribe,
  transcribing,
}: {
  meeting: Meeting;
  currentMs: number;
  onSeek: (ms: number) => void;
  onTranscribe?: () => void;
  transcribing?: boolean;
}) {
  const [query, setQuery] = useState("");
  const [copied, setCopied] = useState(false);
  const activeRef = useRef<HTMLButtonElement>(null);
  const isLive = meeting.status === "live" || meeting.status === "processing";

  const activeId = useMemo(() => {
    const seg = meeting.transcript.find(
      (s) => currentMs >= s.startMs && currentMs < s.endMs,
    );
    return seg?.id;
  }, [meeting.transcript, currentMs]);

  // Auto-scroll: follow the active segment, or the tail while live.
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeId]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return meeting.transcript;
    return meeting.transcript.filter((s) => s.text.toLowerCase().includes(q));
  }, [meeting.transcript, query]);

  const copyAll = async () => {
    const text = meeting.transcript
      .map((s) => `[${formatTimestamp(s.startMs)}] ${s.speaker}: ${s.text}`)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="mb-3 flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search transcript…"
            className="h-9 pl-9"
          />
          {query && (
            <button
              onClick={() => setQuery("")}
              className="absolute right-2 top-1/2 grid size-6 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-secondary"
            >
              <X className="size-4" />
            </button>
          )}
        </div>
        <button
          onClick={copyAll}
          className="no-drag flex h-9 items-center gap-1.5 rounded-md border border-border px-3 text-xs font-medium text-muted-foreground hover:bg-secondary hover:text-foreground"
        >
          {copied ? (
            <Check className="size-4 text-success" />
          ) : (
            <Copy className="size-4" />
          )}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>

      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="space-y-1">
          {filtered.map((seg) => (
            <SegmentRow
              key={seg.id}
              ref={seg.id === activeId ? activeRef : undefined}
              segment={seg}
              active={seg.id === activeId}
              query={query}
              onClick={() => onSeek(seg.startMs)}
            />
          ))}

          {isLive && (
            <div className="flex items-center gap-2 px-2 py-3 text-sm text-muted-foreground">
              <span className="flex gap-1">
                <Dot delay={0} />
                <Dot delay={0.15} />
                <Dot delay={0.3} />
              </span>
              Listening…
            </div>
          )}

          {filtered.length === 0 &&
            !isLive &&
            (query ? (
              <p className="px-2 py-8 text-center text-sm text-muted-foreground">
                No matches in transcript.
              </p>
            ) : onTranscribe ? (
              <div className="flex flex-col items-center gap-3 px-6 py-12 text-center">
                <div className="grid size-12 place-items-center rounded-2xl bg-accent">
                  <ScrollText className="size-5 text-primary" />
                </div>
                <div>
                  <p className="text-sm font-medium">No transcript yet</p>
                  <p className="mt-1 max-w-xs text-xs text-muted-foreground">
                    Generate a transcript from the recording with AssemblyAI —
                    speaker labels &amp; timestamps.
                  </p>
                </div>
                <Button
                  variant="brand"
                  onClick={onTranscribe}
                  disabled={transcribing}
                >
                  {transcribing ? (
                    <Loader2 className="animate-spin" />
                  ) : (
                    <ScrollText />
                  )}
                  {transcribing ? "Transcribing…" : "Transcribe"}
                </Button>
              </div>
            ) : (
              <p className="px-2 py-8 text-center text-sm text-muted-foreground">
                No transcript available.
              </p>
            ))}
        </div>
      </ScrollArea>
    </div>
  );
}

const SegmentRow = forwardRef<
  HTMLButtonElement,
  {
    segment: TranscriptSegment;
    active: boolean;
    query: string;
    onClick: () => void;
  }
>(({ segment, active, query, onClick }, ref) => {
  const color = speakerColor(segment.speaker);
  return (
    <button
      ref={ref}
      onClick={onClick}
      className={cn(
        "no-drag flex w-full gap-3 rounded-lg px-2 py-2 text-left transition-colors",
        active ? "bg-primary/10" : "hover:bg-secondary/60",
      )}
    >
      <span
        className={cn(
          "mt-0.5 grid size-7 shrink-0 place-items-center rounded-full text-[10px] font-bold",
          color,
        )}
      >
        {initials(segment.speaker)}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-xs font-semibold text-foreground">
            {segment.speaker}
          </span>
          <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
            {formatTimestamp(segment.startMs)}
          </span>
        </div>
        <p className="selectable mt-0.5 text-sm leading-relaxed text-foreground/90">
          {highlight(segment.text, query)}
        </p>
      </div>
    </button>
  );
});
SegmentRow.displayName = "SegmentRow";

function highlight(text: string, query: string) {
  const q = query.trim();
  if (!q) return text;
  const idx = text.toLowerCase().indexOf(q.toLowerCase());
  if (idx === -1) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark className="rounded bg-warning/40 text-foreground">
        {text.slice(idx, idx + q.length)}
      </mark>
      {text.slice(idx + q.length)}
    </>
  );
}

function Dot({ delay }: { delay: number }) {
  return (
    <span
      className="size-1.5 animate-bounce rounded-full bg-primary"
      style={{ animationDelay: `${delay}s`, animationDuration: "1s" }}
    />
  );
}

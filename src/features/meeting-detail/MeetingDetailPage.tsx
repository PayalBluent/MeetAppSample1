import { useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { motion } from "motion/react";
import {
  ArrowLeft,
  Bookmark,
  Clock,
  FolderOpen,
  ListChecks,
  Loader2,
  Lock,
  LockOpen,
  MoreHorizontal,
  ScrollText,
  Sparkles,
  Square,
  Star,
  Trash2,
  Waypoints,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { QuickTip } from "@/components/ui/tooltip";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn, formatClock, formatDuration } from "@/lib/utils";
import { PLATFORM_META } from "@/lib/meta";
import { api } from "@/lib/api";
import { toMediaSrc } from "@/lib/media";
import {
  useDeleteMeeting,
  useMeeting,
  useStopCapture,
  useSummarizeMeeting,
  useToggleFlag,
  useTranscribeMeeting,
  useUpdateActionItem,
} from "@/hooks/useMeetings";
import { usePlayback } from "./usePlayback";
import { MediaPlayer } from "./MediaPlayer";
import { TranscriptView } from "./TranscriptView";
import { SummaryView } from "./SummaryView";
import { ActionItems } from "./ActionItems";
import { TimelineView } from "./TimelineView";
import { Participants } from "./Participants";
import type { Meeting } from "@/types";

export function MeetingDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { data: meeting, isLoading } = useMeeting(id);

  const toggleFlag = useToggleFlag();
  const stopCapture = useStopCapture();
  const transcribe = useTranscribeMeeting();
  const summarize = useSummarizeMeeting();
  const updateActionItem = useUpdateActionItem();
  const deleteMeeting = useDeleteMeeting();

  const [mediaEl, setMediaEl] = useState<HTMLMediaElement | null>(null);

  const durationMs = useMemo(() => {
    if (!meeting) return 0;
    const lastEnd = meeting.transcript.reduce(
      (max, s) => Math.max(max, s.endMs),
      0,
    );
    return Math.max(meeting.durationSec * 1000, lastEnd);
  }, [meeting]);

  const playback = usePlayback(durationMs, mediaEl);

  if (isLoading) return <DetailSkeleton />;
  if (!meeting) return <NotFound onBack={() => navigate("/app/meetings")} />;

  const meta = PLATFORM_META[meeting.platform];
  const live = meeting.status === "live";
  const hasMedia = meeting.hasAudio || meeting.hasVideo;
  const audioSrc = toMediaSrc(meeting.audioPath);
  const videoSrc = toMediaSrc(meeting.videoPath);
  // The recording is auto-corrected (noise cancellation + loudness enhancement)
  // while the meeting is "processing"; the player only appears once it's "ready".
  const mediaReady = meeting.status === "ready";
  // Transcription needs captured audio; summarization needs a transcript. When no
  // audio was captured there's nothing to transcribe, and with no transcript
  // there's nothing to summarize — so both actions are disabled in that case.
  const hasTranscript = meeting.transcript.length > 0;
  const canTranscribe = meeting.hasAudio;
  const canSummarize = hasTranscript;

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="shrink-0 border-b border-border px-6 py-4">
        <div className="flex items-start gap-3">
          <Button
            variant="ghost"
            size="icon"
            className="mt-0.5 shrink-0"
            onClick={() => navigate("/app/meetings")}
          >
            <ArrowLeft className="size-5" />
          </Button>

          <span
            className={cn(
              "mt-0.5 grid size-10 shrink-0 place-items-center rounded-xl",
              meta.tint,
            )}
          >
            <meta.icon className={cn("size-5", meta.color)} />
          </span>

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h1 className="truncate text-xl font-bold tracking-tight">
                {meeting.title}
              </h1>
              <StatusBadge meeting={meeting} />
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
              <span>{meta.label}</span>
              <span className="flex items-center gap-1">
                <Clock className="size-3.5" />
                {formatDuration(meeting.durationSec)}
              </span>
              <span>{formatClock(meeting.startedAt)}</span>
              {meeting.tags.map((t) => (
                <span key={t} className="text-primary">
                  #{t}
                </span>
              ))}
            </div>
          </div>

          {/* Actions */}
          <div className="flex shrink-0 items-center gap-1">
            {live && (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => stopCapture.mutate()}
                disabled={stopCapture.isPending}
              >
                <Square className="fill-current" />
                Stop
              </Button>
            )}
            <IconToggle
              label={meeting.isStarred ? "Unstar" : "Star"}
              active={meeting.isStarred}
              onClick={() => toggleFlag.mutate({ id: meeting.id, flag: "starred" })}
            >
              <Star
                className={cn(
                  "size-[18px]",
                  meeting.isStarred && "fill-amber-400 text-amber-400",
                )}
              />
            </IconToggle>
            <IconToggle
              label={meeting.isBookmarked ? "Remove bookmark" : "Bookmark"}
              active={meeting.isBookmarked}
              onClick={() =>
                toggleFlag.mutate({ id: meeting.id, flag: "bookmarked" })
              }
            >
              <Bookmark
                className={cn(
                  "size-[18px]",
                  meeting.isBookmarked && "fill-primary text-primary",
                )}
              />
            </IconToggle>
            <IconToggle
              label={meeting.isLocked ? "Unlock" : "Lock"}
              active={meeting.isLocked}
              onClick={() => toggleFlag.mutate({ id: meeting.id, flag: "locked" })}
            >
              {meeting.isLocked ? (
                <Lock className="size-[18px]" />
              ) : (
                <LockOpen className="size-[18px]" />
              )}
            </IconToggle>

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon">
                  <MoreHorizontal className="size-5" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => api.openRecordingsFolder()}>
                  <FolderOpen />
                  Open recordings folder
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => transcribe.mutate(meeting.id)}
                  disabled={!canTranscribe}
                >
                  <ScrollText />
                  {canTranscribe
                    ? "Transcribe (AssemblyAI)"
                    : "Transcribe — no audio"}
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => summarize.mutate(meeting.id)}
                  disabled={!canSummarize}
                >
                  <Sparkles />
                  {canSummarize
                    ? "Summarize (Groq)"
                    : "Summarize — transcribe first"}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  destructive
                  onSelect={() => {
                    deleteMeeting.mutate(meeting.id);
                    navigate("/app/meetings");
                  }}
                >
                  <Trash2 />
                  Delete meeting
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </div>

      {(transcribe.isError || summarize.isError) && (
        <div className="mx-6 mt-2 shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {String(transcribe.error ?? summarize.error)}
        </div>
      )}

      {/* Body */}
      <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[1fr_360px]">
        {/* Main column: tabs */}
        <div className="flex min-h-0 flex-col border-r border-border">
          <Tabs
            defaultValue={live ? "transcript" : "summary"}
            className="flex min-h-0 flex-1 flex-col"
          >
            <div className="shrink-0 px-6 pt-4">
              <TabsList>
                <TabsTrigger value="summary">
                  <Sparkles /> Summary
                </TabsTrigger>
                <TabsTrigger value="transcript">
                  <ScrollText /> Transcript
                </TabsTrigger>
                <TabsTrigger value="actions">
                  <ListChecks /> Actions
                </TabsTrigger>
                <TabsTrigger value="timeline">
                  <Waypoints /> Timeline
                </TabsTrigger>
              </TabsList>
            </div>

            <TabsContent
              value="summary"
              className="mt-0 min-h-0 flex-1 overflow-hidden"
            >
              <ScrollArea className="h-full">
                <div className="space-y-6 px-6 py-4">
                  <SummaryView
                    meeting={meeting}
                    regenerating={summarize.isPending}
                    canSummarize={canSummarize}
                    onRegenerate={() => summarize.mutate(meeting.id)}
                  />
                  <Separator />
                  <ActionItems
                    meeting={meeting}
                    onToggle={(itemId, done) =>
                      updateActionItem.mutate({
                        meetingId: meeting.id,
                        itemId,
                        done,
                      })
                    }
                  />
                </div>
              </ScrollArea>
            </TabsContent>

            <TabsContent
              value="transcript"
              className="mt-0 min-h-0 flex-1 overflow-hidden px-6 py-4"
            >
              <TranscriptView
                meeting={meeting}
                currentMs={playback.currentMs}
                onSeek={playback.seek}
                onTranscribe={
                  meeting.hasAudio
                    ? () => transcribe.mutate(meeting.id)
                    : undefined
                }
                transcribing={transcribe.isPending}
              />
            </TabsContent>

            <TabsContent
              value="actions"
              className="mt-0 min-h-0 flex-1 overflow-hidden"
            >
              <ScrollArea className="h-full">
                <div className="px-6 py-4">
                  <ActionItems
                    meeting={meeting}
                    onToggle={(itemId, done) =>
                      updateActionItem.mutate({
                        meetingId: meeting.id,
                        itemId,
                        done,
                      })
                    }
                  />
                </div>
              </ScrollArea>
            </TabsContent>

            <TabsContent
              value="timeline"
              className="mt-0 min-h-0 flex-1 overflow-hidden"
            >
              <ScrollArea className="h-full">
                <div className="px-6 py-4">
                  <TimelineView
                    markers={meeting.timeline}
                    currentMs={playback.currentMs}
                    onSeek={playback.seek}
                  />
                </div>
              </ScrollArea>
            </TabsContent>
          </Tabs>
        </div>

        {/* Aside: media + people */}
        <ScrollArea className="min-h-0">
          <div className="space-y-5 p-5">
            {!hasMedia ? (
              <div className="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
                Transcript-only session — no audio was saved.
              </div>
            ) : mediaReady ? (
              <MediaPlayer
                meeting={meeting}
                playback={playback}
                audioSrc={audioSrc}
                videoSrc={videoSrc}
                onMediaEl={setMediaEl}
                onOpenFile={() => api.openRecordingsFolder()}
              />
            ) : (
              // Audio is auto-corrected (noise cancellation + loudness enhancement)
              // before it can be played. Until then it's unavailable — no player.
              <div className="flex items-start gap-3 rounded-xl border border-border bg-secondary/30 p-4 text-sm">
                <Loader2 className="mt-0.5 size-4 shrink-0 animate-spin text-primary" />
                <div>
                  <p className="font-medium text-foreground">
                    Audio unavailable
                  </p>
                  <p className="mt-1 text-muted-foreground">
                    {live
                      ? "Recording in progress — the audio becomes available once it's processed."
                      : "Enhancing and noise-cleaning the audio… it'll be playable as soon as it's ready."}
                  </p>
                </div>
              </div>
            )}

            {meeting.participants.length > 0 && (
              <motion.div
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
              >
                <h3 className="mb-3 text-sm font-semibold">
                  Participants ({meeting.participants.length})
                </h3>
                <Participants people={meeting.participants} />
              </motion.div>
            )}
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}

function StatusBadge({ meeting }: { meeting: Meeting }) {
  if (meeting.status === "live")
    return (
      <Badge variant="destructive" className="gap-1.5">
        <span className="size-1.5 rounded-full bg-destructive animate-rec-pulse" />
        Recording
      </Badge>
    );
  if (meeting.status === "processing")
    return (
      <Badge variant="warning" className="gap-1.5">
        <Sparkles className="size-3" />
        Processing
      </Badge>
    );
  if (meeting.status === "failed")
    return <Badge variant="destructive">Failed</Badge>;
  return null;
}

function IconToggle({
  children,
  label,
  active,
  onClick,
}: {
  children: React.ReactNode;
  label: string;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <QuickTip label={label}>
      <Button
        variant="ghost"
        size="icon"
        onClick={onClick}
        className={cn(active && "text-foreground")}
      >
        {children}
      </Button>
    </QuickTip>
  );
}

function DetailSkeleton() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-3">
        <Skeleton className="size-10 rounded-xl" />
        <div className="space-y-2">
          <Skeleton className="h-5 w-64" />
          <Skeleton className="h-3 w-40" />
        </div>
      </div>
      <div className="mt-8 grid grid-cols-[1fr_340px] gap-6">
        <div className="space-y-3">
          <Skeleton className="h-9 w-72" />
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
        <Skeleton className="h-56 w-full rounded-xl" />
      </div>
    </div>
  );
}

function NotFound({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
      <p className="text-sm text-muted-foreground">Meeting not found.</p>
      <Button variant="outline" onClick={onBack}>
        <ArrowLeft /> Back to meetings
      </Button>
    </div>
  );
}

import { useNavigate } from "react-router-dom";
import { AnimatePresence, motion } from "motion/react";
import {
  AlertTriangle,
  Bot,
  CalendarDays,
  ChevronRight,
  CloudUpload,
  FolderOpen,
  LogOut,
  Mic,
  Settings as SettingsIcon,
  Square,
  User,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { ScrollArea } from "@/components/ui/scroll-area";
import { QuickTip } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn, formatTimestamp } from "@/lib/utils";
import { MODE_META } from "@/lib/meta";
import { win, showMainWindow } from "@/lib/window";
import { api } from "@/lib/api";
import {
  useDetectedMeetings,
  useRecorderStatus,
  useSettings,
  useUpdateSettings,
  useSetMode,
  useStartCapture,
  useStopCapture,
  useCaptureDetected,
  useDismissDetected,
} from "@/hooks/useMeetings";
import { ModeSelector } from "./ModeSelector";
import { AudioMeter } from "./AudioMeter";
import { DetectionCard } from "./DetectionCard";

export function PanelPage() {
  const navigate = useNavigate();
  const { data: status } = useRecorderStatus();
  const { data: settings } = useSettings();
  const { data: detected = [] } = useDetectedMeetings();

  const setMode = useSetMode();
  const updateSettings = useUpdateSettings();
  const startCapture = useStartCapture();
  const stopCapture = useStopCapture();
  const captureDetected = useCaptureDetected();
  const dismissDetected = useDismissDetected();

  const mode = status?.mode ?? "off";
  const recording = status?.state === "recording";
  const processing = status?.state === "processing";

  const openMeetings = async () => {
    const shown = await showMainWindow();
    if (!shown) navigate("/app/meetings");
  };

  return (
    <div className="flex h-screen justify-center overflow-hidden bg-background">
      <div className="flex w-full max-w-[400px] flex-col">
        {/* Drag strip + hide control */}
        <div className="drag-region flex h-8 items-center justify-end px-2">
          <QuickTip label="Hide" side="bottom">
            <button
              onClick={() => win.hide()}
              className="no-drag grid size-6 place-items-center rounded-md text-muted-foreground/60 hover:bg-secondary hover:text-foreground"
            >
              <ChevronRight className="size-4 rotate-90" />
            </button>
          </QuickTip>
        </div>

        <ScrollArea className="flex-1">
          <div className="space-y-5 px-5 pb-7 pt-1">
            {/* Header */}
            <header className="flex items-center justify-between">
              <h1 className="text-[28px] font-bold leading-none tracking-tight">
                MeetApp
              </h1>
              <ProfileMenu
                onSettings={() => navigate("/app/settings")}
                onOpenFolder={() => api.openRecordingsFolder()}
                onQuit={() => win.close()}
              />
            </header>

            {/* Meetings entry */}
            <button
              onClick={openMeetings}
              className="no-drag group flex w-full items-center gap-3 rounded-xl border border-border bg-card px-4 py-3 text-left transition-colors hover:border-primary/40 hover:bg-accent/50"
            >
              <CalendarDays className="size-[18px] text-foreground" />
              <span className="flex-1 text-sm font-medium">Meetings</span>
              <ChevronRight className="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </button>

            <Separator />

            {/* AI Note Taker */}
            <section className="space-y-3">
              <div className="flex items-center gap-2">
                <h2 className="text-lg font-semibold tracking-tight text-primary">
                  AI Note Taker
                </h2>
                <QuickTip label="Import an audio file">
                  <button className="no-drag text-muted-foreground hover:text-foreground">
                    <CloudUpload className="size-4" />
                  </button>
                </QuickTip>
              </div>

              <ModeSelector
                value={mode}
                disabled={recording || processing || setMode.isPending}
                onChange={(m) => setMode.mutate(m)}
              />

              {/* Live detections */}
              <AnimatePresence initial={false}>
                {!recording &&
                  detected.map((d) => (
                    <DetectionCard
                      key={d.id}
                      detection={d}
                      busy={captureDetected.isPending}
                      onCapture={() => captureDetected.mutate(d.id)}
                      onDismiss={() => dismissDetected.mutate(d.id)}
                    />
                  ))}
              </AnimatePresence>

              {/* Live recording bar */}
              <AnimatePresence>
                {recording && status && (
                  <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: "auto" }}
                    exit={{ opacity: 0, height: 0 }}
                    className="overflow-hidden"
                  >
                    <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-3">
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                          <span className="size-2.5 rounded-full bg-destructive animate-rec-pulse" />
                          <span className="text-sm font-semibold text-foreground">
                            {MODE_META[status.mode].label}
                          </span>
                        </div>
                        <span className="font-mono text-sm tabular-nums text-foreground">
                          {formatTimestamp(status.elapsedSec * 1000)}
                        </span>
                      </div>
                      <div className="mt-3 space-y-2">
                        <MeterRow
                          label="Mic"
                          level={status.micLevel}
                          tone="destructive"
                        />
                        {status.mode !== "transcribe" && (
                          <MeterRow label="System" level={status.systemLevel} />
                        )}
                      </div>
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>

              {/* Capture warning (e.g. no audio device / mic permission) */}
              {status?.message && (
                <div className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                  <AlertTriangle className="mt-px size-3.5 shrink-0" />
                  <span>{status.message}</span>
                </div>
              )}

              {/* Primary actions */}
              <div className="grid grid-cols-2 gap-2.5">
                {recording ? (
                  <Button
                    variant="destructive"
                    className="col-span-2"
                    onClick={() => stopCapture.mutate()}
                    disabled={stopCapture.isPending}
                  >
                    <Square className="fill-current" />
                    Stop &amp; save
                  </Button>
                ) : (
                  <>
                    <Button
                      variant="outline"
                      onClick={() =>
                        startCapture.mutate({ title: "Live recording" })
                      }
                      disabled={startCapture.isPending || processing}
                    >
                      <Mic className="text-destructive" />
                      Record Live
                    </Button>
                    <Button
                      variant="outline"
                      onClick={() => api.sendBot()}
                    >
                      <Bot />
                      Send Bot
                    </Button>
                  </>
                )}
              </div>
            </section>

            <Separator />

            {/* Noise cancellation */}
            <section className="space-y-3">
              <h2 className="text-lg font-semibold tracking-tight text-primary">
                Noise Cancellation
              </h2>
              <ToggleRow
                label="Cancel my noise"
                checked={settings?.cancelMyNoise ?? false}
                onChange={(v) => updateSettings.mutate({ cancelMyNoise: v })}
              />
              <ToggleRow
                label="Cancel others' noise"
                checked={settings?.cancelOthersNoise ?? false}
                onChange={(v) =>
                  updateSettings.mutate({ cancelOthersNoise: v })
                }
              />
            </section>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}

function MeterRow({
  label,
  level,
  tone = "primary",
}: {
  label: string;
  level: number;
  tone?: "primary" | "destructive";
}) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-12 shrink-0 text-xs text-muted-foreground">{label}</span>
      <AudioMeter level={level} tone={tone} className="flex-1" />
    </div>
  );
}

function ToggleRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-between rounded-lg px-1 py-1",
      )}
    >
      <span className="text-[15px] font-semibold text-foreground">{label}</span>
      <Switch checked={checked} onCheckedChange={onChange} />
    </div>
  );
}

function ProfileMenu({
  onSettings,
  onOpenFolder,
  onQuit,
}: {
  onSettings: () => void;
  onOpenFolder: () => void;
  onQuit: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button className="no-drag grid size-10 place-items-center rounded-full border border-border bg-secondary/70 text-muted-foreground transition-colors hover:text-foreground">
          <User className="size-5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-52">
        <DropdownMenuLabel>Signed in</DropdownMenuLabel>
        <DropdownMenuItem onSelect={onSettings}>
          <SettingsIcon />
          Settings
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onOpenFolder}>
          <FolderOpen />
          Open recordings folder
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem destructive onSelect={onQuit}>
          <LogOut />
          Quit MeetApp
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

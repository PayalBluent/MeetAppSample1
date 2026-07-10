import { useEffect, useState } from "react";
import {
  FolderOpen,
  Monitor,
  Moon,
  Rocket,
  Sparkles,
  Sun,
  Waves,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { MODE_META, ORDERED_MODES } from "@/lib/meta";
import { api } from "@/lib/api";
import { useSettings, useUpdateSettings } from "@/hooks/useMeetings";
import type { CaptureMode, Settings } from "@/types";

export function SettingsPage() {
  const { data: settings } = useSettings();
  const update = useUpdateSettings();

  if (!settings) return null;
  const set = (patch: Partial<Settings>) => update.mutate(patch);

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto max-w-2xl px-6 py-6">
        <h1 className="text-2xl font-bold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Configure capture defaults, audio, and startup behavior.
        </p>

        {/* Capture */}
        <SettingsSection title="Capture" className="mt-8">
          <Field
            label="Default mode"
            description="Applied automatically when a meeting is detected."
          >
            <div className="flex flex-wrap gap-1.5">
              {ORDERED_MODES.map((mode) => (
                <ModeChip
                  key={mode}
                  mode={mode}
                  active={settings.defaultMode === mode}
                  onClick={() => set({ defaultMode: mode })}
                />
              ))}
            </div>
          </Field>
          <Separator />
          <ToggleField
            label="Auto-record detected meetings"
            description="Start capturing as soon as a call is detected."
            checked={settings.autoRecordDetected}
            onChange={(v) => set({ autoRecordDetected: v })}
          />
        </SettingsSection>

        {/* Audio */}
        <SettingsSection title="Audio" icon={<Waves className="size-4" />}>
          <ToggleField
            label="Record computer audio"
            description="Capture your speaker output (the other participants) along with your microphone. Windows only."
            checked={settings.captureSystemAudio}
            onChange={(v) => set({ captureSystemAudio: v })}
          />
          <Separator />
          <ToggleField
            label="Cancel my noise"
            description="Suppress background noise from your microphone."
            checked={settings.cancelMyNoise}
            onChange={(v) => set({ cancelMyNoise: v })}
          />
          <Separator />
          <ToggleField
            label="Cancel others' noise"
            description="Clean up noise coming from other participants."
            checked={settings.cancelOthersNoise}
            onChange={(v) => set({ cancelOthersNoise: v })}
          />
        </SettingsSection>

        {/* Startup */}
        <SettingsSection title="Startup" icon={<Rocket className="size-4" />}>
          <ToggleField
            label="Launch on startup"
            description="Open MeetApp automatically when you sign in."
            checked={settings.launchOnStartup}
            onChange={(v) => set({ launchOnStartup: v })}
          />
          <Separator />
          <ToggleField
            label="Start minimized to tray"
            description="Run in the background without opening a window."
            checked={settings.startMinimized}
            onChange={(v) => set({ startMinimized: v })}
          />
        </SettingsSection>

        {/* Storage */}
        <SettingsSection title="Storage">
          <Field
            label="Recordings folder"
            description="Where audio and video files are saved."
          >
            <div className="flex items-center gap-2">
              <code className="flex-1 truncate rounded-md border border-border bg-muted px-3 py-1.5 text-xs text-muted-foreground">
                {settings.saveDirectory}
              </code>
              <Button
                variant="outline"
                size="sm"
                onClick={() => api.openRecordingsFolder()}
              >
                <FolderOpen />
                Open
              </Button>
            </div>
          </Field>
        </SettingsSection>

        {/* AI services */}
        <SettingsSection
          title="AI Services"
          icon={<Sparkles className="size-4" />}
        >
          <TextField
            label="AssemblyAI API key"
            description="Cloud transcription with speaker labels & timestamps. Get a key at assemblyai.com. Tip: set ASSEMBLYAI_API_KEY in the project .env to prefill this."
            value={settings.assemblyaiApiKey}
            placeholder="AssemblyAI key"
            secret
            onCommit={(v) => set({ assemblyaiApiKey: v })}
          />
          <Separator />
          <TextField
            label="Groq API key"
            description="Fast LLM summaries & action items. Get a key at console.groq.com. Tip: set GROQ_API_KEY in the project .env to prefill this."
            value={settings.groqApiKey}
            placeholder="gsk_…"
            secret
            onCommit={(v) => set({ groqApiKey: v })}
          />
          <Separator />
          <TextField
            label="Groq model"
            description="Change if a model is retired (see console.groq.com/docs/models)."
            value={settings.groqModel}
            placeholder="llama-3.3-70b-versatile"
            onCommit={(v) => set({ groqModel: v })}
          />
        </SettingsSection>

        {/* Appearance */}
        <SettingsSection title="Appearance">
          <Field label="Theme">
            <div className="grid grid-cols-3 gap-2">
              <ThemeChip
                label="Light"
                icon={<Sun className="size-4" />}
                active={settings.theme === "light"}
                onClick={() => set({ theme: "light" })}
              />
              <ThemeChip
                label="Dark"
                icon={<Moon className="size-4" />}
                active={settings.theme === "dark"}
                onClick={() => set({ theme: "dark" })}
              />
              <ThemeChip
                label="System"
                icon={<Monitor className="size-4" />}
                active={settings.theme === "system"}
                onClick={() => set({ theme: "system" })}
              />
            </div>
          </Field>
        </SettingsSection>

        <div className="h-8" />
      </div>
    </ScrollArea>
  );
}

function SettingsSection({
  title,
  icon,
  children,
  className,
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mt-6", className)}>
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold text-muted-foreground">
        {icon}
        {title}
      </h2>
      <div className="divide-y divide-border rounded-xl border border-border bg-card">
        {children}
      </div>
    </section>
  );
}

function Field({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:justify-between">
      <div>
        <p className="text-sm font-medium">{label}</p>
        {description && (
          <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
        )}
      </div>
      <div className="sm:max-w-[60%]">{children}</div>
    </div>
  );
}

function ToggleField({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <Field label={label} description={description}>
      <Switch checked={checked} onCheckedChange={onChange} />
    </Field>
  );
}

/** Text/secret input that commits on blur (avoids a mutation per keystroke). */
function TextField({
  label,
  description,
  value,
  placeholder,
  secret,
  onCommit,
}: {
  label: string;
  description?: string;
  value: string;
  placeholder?: string;
  secret?: boolean;
  onCommit: (v: string) => void;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <Field label={label} description={description}>
      <Input
        type={secret ? "password" : "text"}
        placeholder={placeholder}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => {
          if (v !== value) onCommit(v.trim());
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
        autoComplete="off"
        spellCheck={false}
        className="font-mono text-xs"
      />
    </Field>
  );
}

function ModeChip({
  mode,
  active,
  onClick,
}: {
  mode: CaptureMode;
  active: boolean;
  onClick: () => void;
}) {
  const meta = MODE_META[mode];
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "border-border text-muted-foreground hover:bg-secondary",
      )}
    >
      <meta.icon className="size-3.5" />
      {meta.label}
    </button>
  );
}

function ThemeChip({
  label,
  icon,
  active,
  onClick,
}: {
  label: string;
  icon: React.ReactNode;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag flex items-center justify-center gap-2 rounded-lg border py-2.5 text-sm font-medium transition-colors",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "border-border text-muted-foreground hover:bg-secondary",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

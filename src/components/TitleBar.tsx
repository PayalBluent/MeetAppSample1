import { Minus, Square, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { win } from "@/lib/window";

interface TitleBarProps {
  /** Left-aligned content (title, breadcrumbs). Sits inside the drag region. */
  children?: React.ReactNode;
  className?: string;
  /** Panel windows only get a close button; the main window gets all three. */
  variant?: "main" | "panel";
}

/**
 * Custom window chrome. The whole bar is a drag region; interactive controls
 * opt out via `.no-drag`. Controls call Tauri window APIs (no-op in browser).
 */
export function TitleBar({
  children,
  className,
  variant = "main",
}: TitleBarProps) {
  return (
    <div
      className={cn(
        "drag-region flex h-11 shrink-0 items-center justify-between gap-2 border-b border-border bg-titlebar/80 px-3 backdrop-blur",
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-2">{children}</div>

      <div className="no-drag flex items-center">
        {variant === "main" && (
          <>
            <WindowButton onClick={() => win.minimize()} label="Minimize">
              <Minus className="size-4" />
            </WindowButton>
            <WindowButton
              onClick={() => win.toggleMaximize()}
              label="Maximize"
            >
              <Square className="size-3" />
            </WindowButton>
          </>
        )}
        <WindowButton
          onClick={() => (variant === "panel" ? win.hide() : win.close())}
          label="Close"
          danger
        >
          <X className="size-4" />
        </WindowButton>
      </div>
    </div>
  );
}

function WindowButton({
  children,
  onClick,
  label,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  label: string;
  danger?: boolean;
}) {
  return (
    <button
      aria-label={label}
      onClick={onClick}
      className={cn(
        "flex h-8 w-10 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground",
        danger && "hover:bg-destructive hover:text-destructive-foreground",
      )}
    >
      {children}
    </button>
  );
}

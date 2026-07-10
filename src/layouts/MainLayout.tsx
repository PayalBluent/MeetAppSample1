import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { motion } from "motion/react";
import { CalendarDays, PanelLeftClose, Settings, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { TitleBar } from "@/components/TitleBar";
import { RecorderStatusPill } from "@/components/RecorderStatusPill";
import { QuickTip } from "@/components/ui/tooltip";

const NAV = [
  { to: "/app/meetings", label: "Meetings", icon: CalendarDays },
  { to: "/app/settings", label: "Settings", icon: Settings },
];

export function MainLayout() {
  const navigate = useNavigate();

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background">
      <TitleBar variant="main">
        <button
          onClick={() => navigate("/app/meetings")}
          className="no-drag flex items-center gap-2 rounded-md px-1.5 py-1"
        >
          <span className="grid size-6 place-items-center rounded-md bg-brand text-white shadow-sm">
            <Sparkles className="size-3.5" />
          </span>
          <span className="text-sm font-semibold tracking-tight">MeetApp</span>
        </button>
        <div className="ml-2">
          <RecorderStatusPill />
        </div>
      </TitleBar>

      <div className="flex min-h-0 flex-1">
        {/* Slim nav rail */}
        <nav className="flex w-16 shrink-0 flex-col items-center gap-1 border-r border-border bg-card/40 py-3">
          {NAV.map((item) => (
            <QuickTip key={item.to} label={item.label} side="right">
              <NavLink
                to={item.to}
                className={({ isActive }) =>
                  cn(
                    "no-drag group relative grid size-11 place-items-center rounded-xl text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground",
                    isActive && "text-primary",
                  )
                }
              >
                {({ isActive }) => (
                  <>
                    {isActive && (
                      <motion.span
                        layoutId="rail-active"
                        className="absolute inset-0 rounded-xl bg-primary/10"
                        transition={{ type: "spring", stiffness: 400, damping: 32 }}
                      />
                    )}
                    <item.icon className="relative size-5" />
                  </>
                )}
              </NavLink>
            </QuickTip>
          ))}

          <div className="mt-auto">
            <QuickTip label="Back to panel" side="right">
              <button
                onClick={() => navigate("/")}
                className="no-drag grid size-11 place-items-center rounded-xl text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
              >
                <PanelLeftClose className="size-5" />
              </button>
            </QuickTip>
          </div>
        </nav>

        <main className="min-w-0 flex-1 overflow-hidden">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

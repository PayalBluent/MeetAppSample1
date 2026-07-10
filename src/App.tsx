import { useEffect } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import {
  HashRouter,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { TooltipProvider } from "@/components/ui/tooltip";
import { queryClient } from "@/lib/query";
import { currentWindowLabel } from "@/lib/tauri";
import { useApplyTheme } from "@/hooks/useTheme";
import { useBackendEvents } from "@/hooks/useBackendEvents";
import { MainLayout } from "@/layouts/MainLayout";
import { PanelPage } from "@/features/panel/PanelPage";
import { MeetingsPage } from "@/features/meetings/MeetingsPage";
import { MeetingDetailPage } from "@/features/meeting-detail/MeetingDetailPage";
import { SettingsPage } from "@/features/settings/SettingsPage";

/**
 * Under Tauri the "main" window should open on the meetings list and the
 * "panel" window on the control panel. This runs once to place each window on
 * the right route regardless of how its initial URL was resolved.
 */
function useWindowRouting() {
  const navigate = useNavigate();
  const location = useLocation();
  useEffect(() => {
    let cancelled = false;
    currentWindowLabel().then((label) => {
      if (cancelled) return;
      if (label === "main" && location.pathname === "/") {
        navigate("/app/meetings", { replace: true });
      }
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}

function AppInner() {
  useApplyTheme();
  useBackendEvents();
  useWindowRouting();

  return (
    <Routes>
      {/* Compact tray control panel (the "MeetApp" popover). */}
      <Route path="/" element={<PanelPage />} />

      {/* Main window: meetings list → detail. */}
      <Route path="/app" element={<MainLayout />}>
        <Route index element={<Navigate to="meetings" replace />} />
        <Route path="meetings" element={<MeetingsPage />} />
        <Route path="meetings/:id" element={<MeetingDetailPage />} />
        <Route path="settings" element={<SettingsPage />} />
      </Route>

      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={200} skipDelayDuration={300}>
        <HashRouter>
          <AppInner />
        </HashRouter>
      </TooltipProvider>
    </QueryClientProvider>
  );
}

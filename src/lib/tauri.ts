import type { AppEventName, AppEvents } from "@/types";
import { mockBackend } from "@/mock/backend";

/** True when running inside the Tauri webview (v2 exposes `__TAURI_INTERNALS__`). */
export function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

export type Unlisten = () => void;

/**
 * Unified command invocation. Under Tauri it forwards to the real IPC bridge;
 * otherwise it uses the in-memory mock so the app runs in any browser.
 */
export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauri()) {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return tauriInvoke<T>(cmd, args);
  }
  return mockBackend.invoke<T>(cmd, args);
}

/** Subscribe to a typed backend event. Returns an unlisten function. */
export async function listen<E extends AppEventName>(
  event: E,
  handler: (payload: AppEvents[E]) => void,
): Promise<Unlisten> {
  if (isTauri()) {
    const { listen: tauriListen } = await import("@tauri-apps/api/event");
    const unlisten = await tauriListen<AppEvents[E]>(event, (e) =>
      handler(e.payload),
    );
    return unlisten;
  }
  return mockBackend.bus.on(event, (p) => handler(p as AppEvents[E]));
}

/** Which webview window we're rendering (Tauri sets a distinct label per window). */
export async function currentWindowLabel(): Promise<string> {
  if (isTauri()) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow().label;
  }
  return "browser";
}

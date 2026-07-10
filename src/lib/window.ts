import { isTauri } from "./tauri";

/**
 * Thin, browser-safe wrappers around Tauri window controls. Every function is a
 * no-op (or console log) when not running under Tauri, so the UI works in a
 * plain browser during development.
 */

async function withWindow<T>(
  fn: (win: import("@tauri-apps/api/window").Window) => Promise<T>,
): Promise<T | undefined> {
  if (!isTauri()) return undefined;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return fn(getCurrentWindow());
}

export const win = {
  minimize: () => withWindow((w) => w.minimize()),
  toggleMaximize: () => withWindow((w) => w.toggleMaximize()),
  close: () => withWindow((w) => w.close()),
  hide: () => withWindow((w) => w.hide()),
  startDragging: () => withWindow((w) => w.startDragging()),
};

/**
 * Show/focus the main window (from the tray panel). Under Tauri this targets the
 * separate "main" window; in the browser there is only one document, so callers
 * fall back to in-app navigation.
 */
export async function showMainWindow(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const main = await WebviewWindow.getByLabel("main");
    if (main) {
      await main.show();
      await main.unminimize();
      await main.setFocus();
      return true;
    }
  } catch (err) {
    console.error("showMainWindow failed", err);
  }
  return false;
}

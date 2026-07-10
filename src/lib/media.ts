import { isTauri } from "./tauri";

/**
 * Convert an on-disk recording path into a URL the webview can play. Under Tauri
 * this uses the asset protocol (`convertFileSrc`); in the browser mock the paths
 * are not real files, so we return undefined and the UI falls back to a
 * synthetic transport.
 */
export function toMediaSrc(path?: string | null): string | undefined {
  if (!path || !isTauri()) return undefined;
  // Lazy require to avoid pulling the API into the browser bundle path.
  // `convertFileSrc` is synchronous but lives in @tauri-apps/api/core.
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const core = (window as any).__TAURI__?.core;
    if (core?.convertFileSrc) return core.convertFileSrc(path) as string;
  } catch {
    /* ignore */
  }
  return undefined;
}

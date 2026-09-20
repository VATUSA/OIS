/**
 * The platform capability layer — the single conditional seam between the web build and the
 * Tauri desktop shell (epic #343).
 *
 * The desktop app renders *this same bundle* inside a Tauri webview (#344), so there is no second
 * frontend and no forked components. Everything that differs between the two goes through here
 * rather than sprinkling `if (desktop)` across the app.
 *
 * Two rules keep the web build honest:
 *
 * 1. **Detection is dependency-free.** `isTauri()` is a plain `in` check against the global Tauri
 *    v2 injects into its webview, so simply asking "am I on desktop?" costs the web build nothing.
 * 2. **`@tauri-apps/api` is only ever reached through a dynamic `import()`** — see
 *    {@link invokeDesktop}. That keeps it in its own chunk, which the web build never loads. A
 *    static `import` at the top of any module would pull it into the main bundle; don't add one.
 */

/** The Tauri v2 runtime injects this into the webview before any app code runs. */
declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

/** True when running inside the Tauri desktop shell, false in a browser. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export type Platform = "desktop" | "web";

/** Which build we're running as. Prefer {@link can} when gating on a specific ability. */
export function platform(): Platform {
  return isTauri() ? "desktop" : "web";
}

/**
 * Desktop abilities the app may ask about. Declared up front so consumers can be written against
 * a stable name before the feature exists — asking about one that isn't built yet is answered
 * honestly rather than being a type error.
 */
export type Capability =
  | "autoUpdate"
  | "notifications"
  | "miniWindows"
  | "multiWindow"
  | "tray"
  | "globalHotkeys"
  | "audioAlerts"
  | "fileDialogs";

/**
 * Whether each capability is actually *implemented* yet, independent of which platform we're on.
 *
 * Every entry starts `false` and is flipped by the issue that builds it, so this module can never
 * claim an ability the app doesn't have. Gating on a capability is therefore safe to write today
 * and starts working the day its feature lands — no caller changes needed.
 */
const IMPLEMENTED: Readonly<Record<Capability, boolean>> = Object.freeze({
  autoUpdate: true, // #347 — signed auto-update (shipped)
  notifications: true, // #348 — native OS notifications (shipped)
  miniWindows: true, // #349 — pop-out always-on-top mini-windows (shipped)
  multiWindow: true, // #350 — multi-window / multi-monitor (shipped)
  tray: true, // #351 — system tray (shipped)
  globalHotkeys: true, // #352 — global hotkeys (shipped)
  audioAlerts: true, // #353 — audio alerts (shipped)
  fileDialogs: false, // #354 — native export/import dialogs
});

const CAPABILITIES = Object.keys(IMPLEMENTED) as Capability[];

/**
 * Every capability and whether it's available *right now*: it needs both the desktop shell and a
 * shipped implementation. On the web build they are all false, always.
 */
export function capabilities(): Readonly<Record<Capability, boolean>> {
  return Object.freeze(
    Object.fromEntries(CAPABILITIES.map((c) => [c, can(c)])) as Record<Capability, boolean>,
  );
}

/**
 * Whether one capability is available right now — the usual way to gate behaviour, and the single
 * definition of "available" that {@link capabilities} maps over.
 */
export function can(capability: Capability): boolean {
  return isTauri() && IMPLEMENTED[capability];
}

/**
 * Call a `#[tauri::command]` in the desktop shell.
 *
 * `@tauri-apps/api` is imported lazily so it stays out of the web bundle — the browser build never
 * downloads it, because it can never get past the {@link isTauri} guard to the import.
 *
 * Throws on the web build rather than returning undefined: reaching here in a browser means a
 * caller forgot to gate on {@link can} or {@link isTauri}, and a silent no-op would hide that.
 */
export async function invokeDesktop<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri()) {
    throw new Error(
      `invokeDesktop(${JSON.stringify(command)}) was called on the web build — gate it behind can() or isTauri()`,
    );
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

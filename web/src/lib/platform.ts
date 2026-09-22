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

import {IMPLEMENTED} from "./platform-flags";

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
  | "notifications"
  | "miniWindows"
  | "multiWindow"
  | "tray"
  | "globalHotkeys"
  | "audioAlerts"
  | "fileDialogs";

// What is implemented lives in `platform-flags.ts`, so a test can turn a flag on (VATUSA/OIS#345).

const CAPABILITIES = Object.keys(IMPLEMENTED) as Capability[];

/**
 * Every capability and whether it's available *right now*: it needs both the desktop shell and a
 * shipped implementation. On the web build they are all false, always.
 *
 * Memoised per platform. `isTauri()` and {@link IMPLEMENTED} are both constant for the life of the
 * process, so a fresh object per call only churned identity — a caller putting the result in a
 * `useMemo`/`useEffect` dependency array would re-run on every render.
 */
const SNAPSHOTS = new Map<boolean, Readonly<Record<Capability, boolean>>>();

export function capabilities(): Readonly<Record<Capability, boolean>> {
  const onDesktop = isTauri();
  let snapshot = SNAPSHOTS.get(onDesktop);
  if (!snapshot) {
    snapshot = Object.freeze(
      Object.fromEntries(
        CAPABILITIES.map((c) => [c, availability(onDesktop, IMPLEMENTED[c])]),
      ) as Record<Capability, boolean>,
    );
    SNAPSHOTS.set(onDesktop, snapshot);
  }
  return snapshot;
}

/**
 * The rule itself: a capability needs *both* the desktop shell and a shipped implementation.
 *
 * Extracted as a pure function so the desktop half of the gate is testable today. Inlined into
 * `can()` it was unobservable — every entry in {@link IMPLEMENTED} is currently `false`, so
 * dropping the `isTauri()` check entirely left the whole suite green while quietly arming a leak
 * of desktop-only UI into the browser the moment any feature issue flips its flag.
 */
export function availability(onDesktop: boolean, implemented: boolean): boolean {
  return onDesktop && implemented;
}

/**
 * Whether one capability is available right now — the usual way to gate behaviour, and the single
 * definition of "available" that {@link capabilities} maps over.
 */
export function can(capability: Capability): boolean {
  return availability(isTauri(), IMPLEMENTED[capability]);
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

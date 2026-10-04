/**
 * The webview half of the desktop log (#629).
 *
 * Installed once, first thing in `bootstrap()`, so a failure during launch is caught too. It keeps
 * `console.*` working exactly as before and additionally records every call — plus uncaught errors,
 * unhandled promise rejections and React render crashes (`components/error-boundary.tsx`) — into:
 *
 * - an in-memory tail, which a diagnostics report carries (`lib/diagnostics.ts`); and
 * - on the desktop, the app's log file, through the log plugin's `log` command. Rust redacts every
 *   line before it is written (`desktop/src-tauri/src/logging.rs`), so nothing here needs to know
 *   which strings are secrets.
 *
 * On the web build only the tail is kept; it never leaves the page.
 */

import {invokeDesktop, isTauri} from "./platform";

export type LogLevel = "debug" | "info" | "warn" | "error";

/** The log plugin's numeric levels (`tauri_plugin_log::LogLevel`). */
const PLUGIN_LEVEL: Record<LogLevel, number> = {debug: 2, info: 3, warn: 4, error: 5};

/** How many recent lines the tail holds. */
export const TAIL_LINES = 200;

const tail: string[] = [];
let installed = false;

/** The most recent log lines, oldest first. */
export function logTail(): string[] {
  return [...tail];
}

/** Records one line in the tail and, on the desktop, in the log file. Never throws. */
export function log(level: LogLevel, message: string, location?: string): void {
  const where = location ? `[${location}]` : "";
  tail.push(`${new Date().toISOString()} [${level}]${where} ${message}`);
  if (tail.length > TAIL_LINES) tail.splice(0, tail.length - TAIL_LINES);
  if (isTauri()) {
    // Swallowed: reporting a logging failure through `console` would log it again, forever.
    invokeDesktop("plugin:log|log", {level: PLUGIN_LEVEL[level], message, location}).catch(() => {});
  }
}

/** One `console` argument as text: an Error's stack, a string as-is, anything else as JSON. */
export function describe(value: unknown): string {
  if (value instanceof Error) return value.stack ?? `${value.name}: ${value.message}`;
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

const CONSOLE_LEVELS: [keyof Console & ("debug" | "log" | "info" | "warn" | "error"), LogLevel][] = [
  ["debug", "debug"],
  ["log", "info"],
  ["info", "info"],
  ["warn", "warn"],
  ["error", "error"],
];

/** Starts capturing. Safe to call more than once; only the first call installs anything. */
export function installLogger(): void {
  if (installed) return;
  installed = true;
  for (const [method, level] of CONSOLE_LEVELS) {
    const original = console[method].bind(console);
    console[method] = (...args: unknown[]) => {
      original(...args);
      log(level, args.map(describe).join(" "), "console");
    };
  }
  window.addEventListener("error", (event) => {
    const at = event.filename ? ` at ${event.filename}:${event.lineno}:${event.colno}` : "";
    log("error", `uncaught ${event.error ? describe(event.error) : event.message}${at}`, "window");
  });
  window.addEventListener("unhandledrejection", (event) => {
    log("error", `unhandled rejection: ${describe(event.reason)}`, "window");
  });
}

/** Test-only: forget everything so each test starts clean. Does not undo `installLogger`. */
export function resetLogTail(): void {
  tail.length = 0;
}

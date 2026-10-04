// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({invoke: (...a: unknown[]) => invoke(...a)}));

type Logger = typeof import("./logger");

const METHODS = ["debug", "log", "info", "warn", "error"] as const;
let originals: Record<(typeof METHODS)[number], (...a: unknown[]) => void>;

/** A fresh module per case: `installLogger` installs once per module, by design. */
async function freshLogger(): Promise<Logger> {
  vi.resetModules();
  return import("./logger");
}

beforeEach(() => {
  originals = Object.fromEntries(METHODS.map((m) => [m, console[m]])) as typeof originals;
  invoke.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  for (const m of METHODS) console[m] = originals[m];
  delete window.__TAURI_INTERNALS__;
  vi.restoreAllMocks();
});

describe("installLogger", () => {
  it("keeps console working and records each call in the tail", async () => {
    const seen: unknown[][] = [];
    console.warn = (...a: unknown[]) => seen.push(a);
    const logger = await freshLogger();
    logger.installLogger();

    console.warn("feed slow", {retry: 2});

    expect(seen).toEqual([["feed slow", {retry: 2}]]);
    expect(logger.logTail().at(-1)).toMatch(/\[warn\]\[console\] feed slow \{"retry":2\}$/);
  });

  it("records uncaught errors and unhandled rejections", async () => {
    const logger = await freshLogger();
    logger.installLogger();

    window.dispatchEvent(new ErrorEvent("error", {message: "boom", error: new Error("boom")}));
    const rejection = new Event("unhandledrejection") as Event & {reason: unknown};
    rejection.reason = new Error("nope");
    window.dispatchEvent(rejection);

    const tail = logger.logTail().join("\n");
    expect(tail).toMatch(/\[error\]\[window\] uncaught Error: boom/);
    expect(tail).toMatch(/\[error\]\[window\] unhandled rejection: Error: nope/);
  });

  it("keeps only the most recent lines", async () => {
    const logger = await freshLogger();
    for (let i = 0; i < logger.TAIL_LINES + 5; i++) logger.log("info", `line ${i}`);
    const tail = logger.logTail();
    expect(tail).toHaveLength(logger.TAIL_LINES);
    expect(tail[0]).toMatch(/line 5$/);
  });

  it("forwards to the desktop log file through the plugin, with its numeric level", async () => {
    window.__TAURI_INTERNALS__ = {};
    const logger = await freshLogger();

    logger.log("error", "render crash", "react");

    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("plugin:log|log", {
        level: 5,
        message: "render crash",
        location: "react",
      }),
    );
  });

  it("never forwards on the web build", async () => {
    const logger = await freshLogger();
    logger.log("error", "web only");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(invoke).not.toHaveBeenCalled();
  });
});

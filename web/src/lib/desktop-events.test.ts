import {afterEach, describe, expect, it, vi} from "vitest";

import {safeUnlisten} from "./desktop-events";

/**
 * Tearing down a Tauri listener must not leave an unhandled rejection behind (#426).
 *
 * The handle `listen()` resolves to is an async function, so a failure inside it — typically
 * `unregisterListener` reading `listeners[eventId].handlerId` for an id the webview has forgotten —
 * arrives as a rejected promise rather than an exception. Discarding it is what produced
 * `[Unhandled rejection] TypeError: undefined is not an object` on unmount.
 */

/** Unhandled rejections are reported once the microtask queue has drained, so wait a macrotask. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function rejectionsDuring(run: () => void): Promise<unknown[]> {
  const seen: unknown[] = [];
  const onUnhandled = (reason: unknown) => seen.push(reason);
  process.on("unhandledRejection", onUnhandled);
  try {
    run();
    await settle();
  } finally {
    process.off("unhandledRejection", onUnhandled);
  }
  return seen;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("safeUnlisten", () => {
  it("runs the handle", () => {
    const unlisten = vi.fn();
    safeUnlisten(unlisten);
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("does nothing when there is no handle", () => {
    expect(() => safeUnlisten(undefined)).not.toThrow();
  });

  it("swallows a rejecting handle without leaving it unhandled", async () => {
    // Deliberately *not* `vi.fn`: the mock attaches its own handler to the returned promise so it can
    // record settled results, which marks the rejection handled and makes this assertion vacuous.
    let calls = 0;
    const unlisten = async () => {
      calls += 1;
      throw new TypeError("undefined is not an object (evaluating 'listeners[eventId].handlerId')");
    };

    const seen = await rejectionsDuring(() => safeUnlisten(unlisten));

    expect(seen).toEqual([]);
    // Guards against a vacuous pass: if the call never happened there would be nothing to reject.
    expect(calls).toBe(1);
  });

  it("swallows a handle that throws synchronously", async () => {
    let calls = 0;
    const unlisten = () => {
      calls += 1;
      throw new Error("no window");
    };

    const seen = await rejectionsDuring(() => {
      expect(() => safeUnlisten(unlisten)).not.toThrow();
    });

    expect(seen).toEqual([]);
    expect(calls).toBe(1);
  });
});

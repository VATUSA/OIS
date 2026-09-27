// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

/** Tauri's app-wide event bus: an event emitted in any window reaches listeners in every window. */
const bus = vi.hoisted(() => new Map<string, Set<() => void>>());
vi.mock("@tauri-apps/api/event", () => ({
  emit: async (event: string) => {
    for (const handler of bus.get(event) ?? []) handler();
  },
  listen: async (event: string, handler: () => void) => {
    if (!bus.has(event)) bus.set(event, new Set());
    bus.get(event)!.add(handler);
    return () => bus.get(event)!.delete(handler);
  },
}));

import {dismissAllAlerts, onDismissAllAlerts} from "./alerts";

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  bus.clear();
});

describe("the dismiss-all seam", () => {
  it("tells every mounted alert surface to clear", () => {
    const a = vi.fn();
    const b = vi.fn();
    const offA = onDismissAllAlerts(a);
    const offB = onDismissAllAlerts(b);

    dismissAllAlerts();

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    offA();
    offB();
  });

  it("stops telling one that has unsubscribed", () => {
    // Otherwise an unmounted component keeps being called and React warns about setting state on it.
    const listener = vi.fn();
    const off = onDismissAllAlerts(listener);
    off();

    dismissAllAlerts();

    expect(listener).not.toHaveBeenCalled();
  });

  it("is harmless when nothing is listening", () => {
    expect(() => dismissAllAlerts()).not.toThrow();
  });
});

describe("dismissing across desktop windows (VATUSA/OIS#352 review)", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  // The hotkey fires in the main window, but alerts show in every window; only the main window's
  // cleared, leaving the one the controller could see.
  it("asks every window's alerts to clear, not just this window's", async () => {
    window.__TAURI_INTERNALS__ = {};
    const otherWindow = vi.fn();
    bus.set("ois://alerts/dismiss-all", new Set([otherWindow]));

    dismissAllAlerts();
    await flush();

    expect(otherWindow).toHaveBeenCalled();
  });

  it("clears here when another window asks", async () => {
    window.__TAURI_INTERNALS__ = {};
    const here = vi.fn();
    const off = onDismissAllAlerts(here);
    await flush();

    for (const handler of bus.get("ois://alerts/dismiss-all") ?? []) handler(); // another window emits
    expect(here).toHaveBeenCalled();
    off();
  });
});

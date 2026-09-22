// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {openPopout, popoutLabel} from "./popout";

const WebviewWindow = vi.fn();
const getByLabel = vi.fn();
const availableMonitors = vi.fn();

/** Handlers the created window registered, so a test can simulate a drag. */
const handlers: {moved?: () => void; resized?: () => void} = {};
const outerPosition = vi.fn();
const outerSize = vi.fn();

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  WebviewWindow: Object.assign(
    function (this: unknown, ...args: unknown[]) {
      WebviewWindow(...args);
      return Object.assign(this as object, {
        onMoved: (cb: () => void) => {
          handlers.moved = cb;
          return Promise.resolve(() => undefined);
        },
        onResized: (cb: () => void) => {
          handlers.resized = cb;
          return Promise.resolve(() => undefined);
        },
        outerPosition: () => outerPosition(),
        outerSize: () => outerSize(),
      });
    },
    {getByLabel: (l: string) => getByLabel(l)},
  ),
}));
vi.mock("@tauri-apps/api/window", () => ({availableMonitors: () => availableMonitors()}));

function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

/**
 * This project's jsdom environment provides no `localStorage`, so the geometry tests supply one.
 * The source survives its absence either way — every access is wrapped — but without a store there
 * would be nothing to assert about remembering where a window was.
 */
function installStorage() {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  });
}

const SPEC = {id: "fca-abc", title: "ATL · metering", route: "/popout/fca/abc"};

beforeEach(() => {
  WebviewWindow.mockReset();
  getByLabel.mockReset().mockResolvedValue(null);
  availableMonitors.mockReset().mockResolvedValue([
    {position: {x: 0, y: 0}, size: {width: 1512, height: 982}},
  ]);
  installStorage();
  handlers.moved = undefined;
  handlers.resized = undefined;
  outerPosition.mockReset().mockResolvedValue({x: 300, y: 400});
  outerSize.mockReset().mockResolvedValue({width: 420, height: 640});
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  vi.unstubAllGlobals();
});

describe("popoutLabel", () => {
  it("makes a window label safe from an arbitrary panel id", () => {
    const label = popoutLabel("widget-9f3a/b c");

    expect(label).toMatch(/^popout-[a-zA-Z0-9-]+$/);
    expect(label.startsWith("popout-widget-9f3a-b-c")).toBe(true);
  });

  it("is stable for the same id, so reopening finds the same window", () => {
    expect(popoutLabel("fca-abc")).toBe(popoutLabel("fca-abc"));
  });
});

describe("openPopout", () => {
  it("does nothing at all on the web build", async () => {
    await expect(openPopout(SPEC)).resolves.toBe(false);
    expect(WebviewWindow).not.toHaveBeenCalled();
  });

  it("opens a window pinned above other apps, at the bare embed route", async () => {
    pretendDesktop();

    await expect(openPopout(SPEC)).resolves.toBe(true);
    expect(WebviewWindow).toHaveBeenCalledWith(
      "popout-fca-abc",
      expect.objectContaining({
        url: "/popout/fca/abc?embed=1",
        title: "ATL · metering",
        alwaysOnTop: true,
      }),
    );
  });

  it("raises the existing window instead of opening a second one", async () => {
    // Clicking "pop out" twice must not leave two identical windows fighting for screen space.
    pretendDesktop();
    const existing = {
      unminimize: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      setFocus: vi.fn().mockResolvedValue(undefined),
    };
    getByLabel.mockResolvedValue(existing);

    await expect(openPopout(SPEC)).resolves.toBe(true);
    expect(WebviewWindow).not.toHaveBeenCalled();
    expect(existing.setFocus).toHaveBeenCalled();
  });

  it("restores saved geometry when the screen it was on still exists", async () => {
    pretendDesktop();
    localStorage.setItem(
      "ois.popout.fca-abc",
      JSON.stringify({x: 100, y: 120, width: 400, height: 600}),
    );

    await openPopout(SPEC);

    expect(WebviewWindow).toHaveBeenCalledWith(
      "popout-fca-abc",
      expect.objectContaining({x: 100, y: 120, width: 400, height: 600}),
    );
  });

  it("does not restore a position that is now off every screen", async () => {
    // Saved on a second monitor that has since been unplugged — restoring it verbatim would put
    // the window somewhere the user cannot reach.
    pretendDesktop();
    localStorage.setItem(
      "ois.popout.fca-abc",
      JSON.stringify({x: 3000, y: 200, width: 400, height: 600}),
    );

    await openPopout(SPEC);

    const options = WebviewWindow.mock.calls[0]![1] as {x?: number};
    expect(options.x).not.toBe(3000);
    expect(options.x).toBeLessThan(1512);
  });

  it("ignores corrupt stored geometry rather than failing to open", async () => {
    pretendDesktop();
    localStorage.setItem("ois.popout.fca-abc", "not json");

    await expect(openPopout(SPEC)).resolves.toBe(true);
  });

  it("appends embed correctly to a route that already has a query", async () => {
    pretendDesktop();
    await openPopout({...SPEC, route: "/popout/fca/abc?tab=x"});

    expect(WebviewWindow).toHaveBeenCalledWith(
      "popout-fca-abc",
      expect.objectContaining({url: "/popout/fca/abc?tab=x&embed=1"}),
    );
  });
});

describe("remembering where a window was put", () => {
  it("writes once after a drag settles, not on every frame of it", async () => {
    // onMoved fires continuously while dragging; writing synchronously to localStorage on each
    // event would mean hundreds of writes for one drag across the screen.
    vi.useFakeTimers();
    try {
      pretendDesktop();
      await openPopout(SPEC);

      for (let i = 0; i < 50; i++) handlers.moved?.();
      expect(localStorage.getItem("ois.popout.fca-abc")).toBeNull();

      await vi.advanceTimersByTimeAsync(400);
      expect(outerPosition).toHaveBeenCalledTimes(1);
      expect(JSON.parse(localStorage.getItem("ois.popout.fca-abc")!)).toEqual({
        x: 300,
        y: 400,
        width: 420,
        height: 640,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("remembers a resize too, not just a move", async () => {
    vi.useFakeTimers();
    try {
      pretendDesktop();
      await openPopout(SPEC);

      handlers.resized?.();
      await vi.advanceTimersByTimeAsync(400);

      expect(localStorage.getItem("ois.popout.fca-abc")).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("popoutLabel", () => {
  it("leaves an already-simple id alone", () => {
    expect(popoutLabel("fca-ZDC")).toBe("popout-fca-ZDC");
  });

  /**
   * A plain character substitution collapsed `ZDC_ARR` and `ZDC.ARR` onto one label, and since the
   * label is what `getByLabel` raises, opening the second panel would have surfaced the first.
   */
  it("keeps ids distinct that flatten to the same characters", () => {
    const underscore = popoutLabel("fca-ZDC_ARR");
    const dot = popoutLabel("fca-ZDC.ARR");

    expect(underscore).not.toBe(dot);
    for (const label of [underscore, dot]) {
      expect(label.startsWith("popout-")).toBe(true);
      expect(label).toMatch(/^[a-zA-Z0-9-]+$/);
    }
  });
});

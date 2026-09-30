// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The main window's own window buttons (#402, #419).
 *
 * Who draws them depends on the host: macOS keeps its decorations, so the **OS** draws the real traffic
 * lights and the app must draw nothing; Windows and Linux run the window undecorated and get the app's
 * replica. Neither belongs on the web build, where the browser draws its own chrome, nor in a route
 * window (#350), which renders this same shell inside a window that still has a native title bar.
 *
 * `isMacOS` and `isMainWindow` are stubbed from the same handles the window mock reads, so each case
 * turns on the host and the label it claims to turn on.
 */
const platform = vi.hoisted(() => ({windowControls: true, label: "main", macos: false}));
vi.mock("@/lib/platform", () => ({
  can: () => platform.windowControls,
  isMacOS: () => platform.macos,
  isMainWindow: async () => platform.label === "main",
}));

const win = vi.hoisted(() => ({
  minimize: vi.fn(() => Promise.resolve()),
  toggleMaximize: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  isFocused: vi.fn(() => Promise.resolve(true)),
  onFocusChanged: vi.fn((_cb: (event: {payload: boolean}) => void) =>
    Promise.resolve(() => undefined),
  ),
  get label() {
    return platform.label;
  },
}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => win}));

import {useDragRegionProps, WindowControls} from "./window-controls";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});
beforeEach(() => {
  platform.windowControls = true;
  platform.label = "main";
  platform.macos = false;
  for (const [name, fn] of Object.entries(win)) {
    if (name !== "label") (fn as ReturnType<typeof vi.fn>).mockClear();
  }
  win.isFocused.mockResolvedValue(true);
  win.onFocusChanged.mockImplementation(() => Promise.resolve(() => undefined));
});

async function render(node: React.ReactNode = <WindowControls />): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(node);
  });
  return host;
}

const button = (host: HTMLElement, label: string) =>
  host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
const lights = (host: HTMLElement) => host.querySelector<HTMLElement>(".traffic-lights");

describe("WindowControls", () => {
  it("renders nothing on a build without the capability", async () => {
    platform.windowControls = false;
    const host = await render();

    expect(host.innerHTML).toBe("");
    // Nor may it ask the window anything — there is no window.
    expect(win.isFocused).not.toHaveBeenCalled();
  });

  it("renders nothing in a route window, which still has its native title bar", async () => {
    // Every Tauri webview runs this same bundle, so a capability-only gate drew a second set of
    // controls over the OS's in every route window (#402 review).
    platform.label = "window--ops-idst";
    const host = await render();

    expect(host.innerHTML).toBe("");
  });

  /**
   * #419: on macOS the real traffic lights are already in this spot — the OS draws them, inset into the
   * app's chrome row by `trafficLightPosition` in `tauri.macos.conf.json`. Drawing the replica as well
   * would stack two sets of buttons on top of each other.
   */
  it("draws no replica on macOS, where the OS draws the real lights", async () => {
    platform.macos = true;
    const host = await render();

    expect(host.innerHTML).toBe("");
    expect(win.isFocused).not.toHaveBeenCalled();
  });

  it("draws the three lights on Windows, and each one acts on the window", async () => {
    const host = await render();

    expect(lights(host)).not.toBeNull();
    expect(host.querySelectorAll("button")).toHaveLength(3);

    await act(async () => button(host, "Close")!.click());
    expect(win.close).toHaveBeenCalledTimes(1);

    await act(async () => button(host, "Minimize")!.click());
    expect(win.minimize).toHaveBeenCalledTimes(1);

    // macOS calls it zoom, so the replica does too — the label is what a screen reader reads out.
    await act(async () => button(host, "Zoom")!.click());
    expect(win.toggleMaximize).toHaveBeenCalledTimes(1);
  });

  it("opts every control out of the drag region, or a click would drag the window", async () => {
    const host = await render();

    const controls = [...host.querySelectorAll("button")];
    expect(controls).toHaveLength(3);
    for (const control of controls) {
      expect(control.getAttribute("data-tauri-drag-region")).toBe("false");
    }
  });

  /**
   * The real buttons grey out when their window loses focus, so the replica has to as well — otherwise
   * a window in the background looks like the active one.
   */
  it("greys out when the window loses focus, and lights up when it comes back", async () => {
    let notify: ((event: {payload: boolean}) => void) | undefined;
    win.onFocusChanged.mockImplementation((cb) => {
      notify = cb;
      return Promise.resolve(() => undefined);
    });
    const host = await render();

    expect(lights(host)!.hasAttribute("data-blurred")).toBe(false);

    await act(async () => notify!({payload: false}));
    expect(lights(host)!.hasAttribute("data-blurred")).toBe(true);

    await act(async () => notify!({payload: true}));
    expect(lights(host)!.hasAttribute("data-blurred")).toBe(false);
  });

  it("starts from the window's real focus state rather than assuming it has focus", async () => {
    win.isFocused.mockResolvedValue(false);
    const host = await render();

    expect(lights(host)!.hasAttribute("data-blurred")).toBe(true);
  });

  it("stops listening for focus when it unmounts", async () => {
    const stop = vi.fn();
    win.onFocusChanged.mockImplementation(() => Promise.resolve(stop));
    await render();

    await act(async () => root?.unmount());
    root = undefined;

    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("useDragRegionProps", () => {
  function Probe() {
    const props = useDragRegionProps();
    return <div data-testid="probe" {...props} />;
  }
  const probe = (host: HTMLElement) => host.querySelector<HTMLElement>('[data-testid="probe"]')!;

  it("marks the row a drag region in the main window, and adds nothing else", async () => {
    const host = await render(<Probe />);

    expect(probe(host).getAttribute("data-tauri-drag-region")).toBe("true");
    // Nothing else, and in particular no double-click handler: Tauri's own drag.js is injected into
    // every webview and already maximizes on a double-click of a drag region, handling the
    // macOS/Windows difference itself — ours on top of it toggled twice (#402 review).
    expect(probe(host).attributes).toHaveLength(2);
  });

  /**
   * #419: the drag region is **not** gated on the OS the way the replica is. macOS's `Overlay` title bar
   * is transparent and sits over the content, so without this the top of a macOS window would not move
   * the window either — the one thing a title bar has to do.
   */
  it("still marks the row on macOS, where an Overlay title bar needs it too", async () => {
    platform.macos = true;
    const host = await render(<Probe />);

    expect(probe(host).getAttribute("data-tauri-drag-region")).toBe("true");
  });

  it("marks nothing in a route window, which is moved by its own title bar", async () => {
    platform.label = "window--ops-idst";
    const host = await render(<Probe />);

    expect(probe(host).hasAttribute("data-tauri-drag-region")).toBe(false);
  });

  it("marks nothing on the web build, where the browser owns the window", async () => {
    platform.windowControls = false;
    const host = await render(<Probe />);

    expect(probe(host).hasAttribute("data-tauri-drag-region")).toBe(false);
  });
});

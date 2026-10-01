// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The window controls carried by the layouts that render outside `AppShell` (#423).
 *
 * Same gate as `WindowControls` itself, deliberately — the bar exists so the signed-out landing page
 * and the error screen are not undecorated rectangles with no way to close them, and it must stay
 * invisible everywhere the OS already draws a title bar.
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

import {WindowChromeBar} from "./window-chrome-bar";

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
  win.isFocused.mockResolvedValue(true);
  win.onFocusChanged.mockImplementation(() => Promise.resolve(() => undefined));
});

async function render(): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(<WindowChromeBar />);
  });
  return host;
}

describe("WindowChromeBar", () => {
  it("offers close, minimize and zoom in the undecorated main window", async () => {
    const host = await render();

    // macOS calls it Zoom, and so does the replica, because the label is its accessible name (#419).
    for (const label of ["Minimize", "Zoom", "Close"]) {
      expect(host.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    }
  });

  /**
   * #419: on macOS the OS draws the real traffic lights over this spot, so the strip stays — it is
   * what makes the top of the window draggable under an `Overlay` title bar — but draws no replica.
   */
  it("keeps the drag strip on macOS but draws no replica in it", async () => {
    platform.macos = true;
    const host = await render();

    expect(host.querySelector("[data-tauri-drag-region]")).not.toBeNull();
    expect(host.querySelectorAll("button")).toHaveLength(0);
  });

  /**
   * The buttons must not move when the user signs in or out, so the strip's leading edge and height
   * match the shell's chrome row rather than being chosen here (#423 review).
   */
  it("places the buttons where the shell does, so they do not jump on sign-in", async () => {
    const host = await render();
    const bar = host.querySelector("[data-tauri-drag-region]")!;

    // `px-2.5` = the sidebar header's own padding; `h-11` = Shell's top bar.
    expect(bar.className).toContain("px-2.5");
    expect(bar.className).toContain("h-11");
    // The width of the buttons is the slot's business, not this strip's — it differs per platform.
    expect(host.querySelector("[data-window-chrome-slot]")).not.toBeNull();
  });

  it("is the window's drag region, so the bar still moves an undecorated window", async () => {
    const host = await render();

    const bar = host.querySelector("[data-tauri-drag-region]");
    expect(bar).not.toBeNull();
    // The bar itself drags; the buttons inside opt out, or a click would drag instead of press.
    expect(bar!.getAttribute("data-tauri-drag-region")).toBe("true");
  });

  it("renders nothing on the web build, where the browser draws its own chrome", async () => {
    platform.windowControls = false;
    const host = await render();

    expect(host.innerHTML).toBe("");
  });

  it("renders nothing in a route window, which still has its native title bar", async () => {
    // `decorations: false` is set on `main` alone, but every Tauri webview runs this same bundle —
    // so a capability-only gate would draw a second set of controls over the OS's own (#350).
    platform.label = "window--ops-idst";
    const host = await render();

    expect(host.innerHTML).toBe("");
  });
});

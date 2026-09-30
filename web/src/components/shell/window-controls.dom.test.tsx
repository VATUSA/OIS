// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The window controls for the frameless main window (#402).
 *
 * The native title bar is off, so these are the only in-app way to manage the window — and they must
 * appear *only* there: not on the web build, where the browser draws its own, and not in a route
 * window (#350), which renders this same shell inside a window that still has its native title bar.
 */
const platform = vi.hoisted(() => ({windowControls: true, label: "main"}));
// `isMainWindow` is stubbed from the same `platform.label` handle the window mock reads, so the
// route-window case below still turns on the label it always did (#403 moved the comparison into
// `platform.ts`, and this factory mock replaces that module wholesale).
vi.mock("@/lib/platform", () => ({
  can: () => platform.windowControls,
  isMainWindow: async () => platform.label === "main",
}));

const win = vi.hoisted(() => ({
  minimize: vi.fn(() => Promise.resolve()),
  toggleMaximize: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  isMaximized: vi.fn(() => Promise.resolve(false)),
  onResized: vi.fn((_cb: () => void) => Promise.resolve(() => undefined)),
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
  for (const [name, fn] of Object.entries(win)) {
    if (name !== "label") (fn as ReturnType<typeof vi.fn>).mockClear();
  }
  win.isMaximized.mockResolvedValue(false);
  win.onResized.mockImplementation(() => Promise.resolve(() => undefined));
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

describe("WindowControls", () => {
  it("renders nothing on a build without the capability", async () => {
    platform.windowControls = false;
    const host = await render();

    expect(host.innerHTML).toBe("");
    // Nor may it ask the window anything — there is no window.
    expect(win.isMaximized).not.toHaveBeenCalled();
  });

  it("renders nothing in a route window, which still has its native title bar", async () => {
    // `decorations: false` is set on `main` alone, but every Tauri webview runs this same bundle, so
    // a capability-only gate drew a second set of controls over the OS's in every route window.
    platform.label = "window--ops-idst";
    const host = await render();

    expect(host.innerHTML).toBe("");
    expect(win.isMaximized).not.toHaveBeenCalled();
  });

  it("minimizes, toggles and closes the window", async () => {
    const host = await render();

    await act(async () => button(host, "Minimize")!.click());
    expect(win.minimize).toHaveBeenCalledTimes(1);

    await act(async () => button(host, "Maximize")!.click());
    expect(win.toggleMaximize).toHaveBeenCalledTimes(1);

    await act(async () => button(host, "Close")!.click());
    expect(win.close).toHaveBeenCalledTimes(1);
  });

  it("opts every control out of the drag region, or a click would drag the window", async () => {
    const host = await render();

    const controls = [...host.querySelectorAll("button")];
    expect(controls).toHaveLength(3);
    for (const control of controls) {
      expect(control.getAttribute("data-tauri-drag-region")).toBe("false");
    }
  });

  it("names the toggle for what it will do, reading the window rather than assuming", async () => {
    win.isMaximized.mockResolvedValue(true);
    const host = await render();

    expect(button(host, "Restore")).not.toBeNull();
    expect(button(host, "Maximize")).toBeNull();
  });

  it("re-reads the window after acting, so a refused toggle doesn't flip the label", async () => {
    const host = await render();

    // The OS took the toggle: the label has to follow the window, not the click.
    win.isMaximized.mockResolvedValue(true);
    await act(async () => button(host, "Maximize")!.click());
    expect(button(host, "Restore"), "the label should follow the window after a toggle").not.toBeNull();

    // The OS refused it: the label must stay put rather than assume the click landed.
    win.isMaximized.mockResolvedValue(true);
    await act(async () => button(host, "Restore")!.click());
    expect(button(host, "Restore")).not.toBeNull();
  });

  it("resyncs when the window is maximized without us", async () => {
    // Snap, Win+Up, dragging to the top edge, or double-clicking the bar: nothing routes through
    // this component, and aria-label is the button's accessible name.
    const host = await render();
    expect(button(host, "Maximize")).not.toBeNull();

    const onResized = win.onResized.mock.calls[0]?.[0];
    expect(onResized, "the component should subscribe to window resizes").toBeTypeOf("function");

    win.isMaximized.mockResolvedValue(true);
    await act(async () => {
      onResized!();
    });

    expect(button(host, "Restore"), "an external maximize should reach the label").not.toBeNull();
  });

  it("stops listening when it unmounts", async () => {
    const stop = vi.fn();
    win.onResized.mockImplementation(() => Promise.resolve(stop));
    await render();

    await act(async () => root?.unmount());
    root = undefined;

    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("useDragRegionProps", () => {
  let seen: Record<string, unknown> | undefined;
  function Probe() {
    const props = useDragRegionProps();
    seen = props;
    return <div data-testid="probe" {...props} />;
  }
  beforeEach(() => {
    seen = undefined;
  });

  it("marks the row a drag region in the frameless main window, and adds nothing else", async () => {
    const host = await render(<Probe />);
    const probe = host.querySelector('[data-testid="probe"]')!;

    expect(probe.getAttribute("data-tauri-drag-region")).toBe("true");
    // Nothing else, and in particular no double-click handler: Tauri's own drag.js is injected into
    // every webview and already maximizes on a double-click of a drag region, handling the
    // macOS/Windows difference itself. One of ours alongside it toggled maximize twice — dead on
    // Windows, unrestorable on macOS — and, being a bubbled React event, also fired when a button
    // or a breadcrumb inside the row was double-clicked (#402 review).
    expect(Object.keys(seen!)).toEqual(["data-tauri-drag-region"]);
  });

  it("marks nothing on the web build", async () => {
    platform.windowControls = false;
    const host = await render(<Probe />);

    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });

  it("marks nothing in a route window, which is moved by its own title bar", async () => {
    platform.label = "window--ops-idst";
    const host = await render(<Probe />);

    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });
});

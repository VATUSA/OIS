// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The window controls for the frameless main window (#402).
 *
 * The native title bar is off, so these are the only in-app way to manage the window — and on the web
 * build they must not appear at all, where there is no window to manage and the browser draws its own.
 */
const platform = vi.hoisted(() => ({windowControls: true}));
vi.mock("@/lib/platform", () => ({can: () => platform.windowControls}));

const win = vi.hoisted(() => ({
  minimize: vi.fn(() => Promise.resolve()),
  toggleMaximize: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  isMaximized: vi.fn(() => Promise.resolve(false)),
}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => win}));

import {WindowControls} from "./window-controls";

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
  for (const fn of Object.values(win)) fn.mockClear();
  win.isMaximized.mockResolvedValue(false);
});

async function render(): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(<WindowControls />);
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
});

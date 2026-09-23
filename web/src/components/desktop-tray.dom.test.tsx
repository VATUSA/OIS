// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const current = vi.hoisted(() => ({label: "main"}));
const showMainWindow = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: current.label,
    onCloseRequested: vi.fn(async () => () => {}),
    hide: vi.fn(),
  }),
}));
vi.mock("@/lib/tray", () => ({ showMainWindow, syncTray: vi.fn(), removeTray: vi.fn() }));

import {CloseToTray} from "./desktop-tray";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});
afterEach(() => showMainWindow.mockClear());

async function mount(enabled: boolean) {
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(<CloseToTray enabled={enabled} />));
  await act(async () => new Promise((r) => setTimeout(r, 0)));
  act(() => root.unmount());
}

describe("CloseToTray (VATUSA/OIS#351 review)", () => {
  // It mounts in every non-embed window. Unguarded, a route window opening with close-to-tray off
  // (the default) called showMainWindow() and dragged focus back to the main window.
  it("leaves focus alone when a route window opens", async () => {
    current.label = "window--ops-idst";
    await mount(false);
    expect(showMainWindow).not.toHaveBeenCalled();
  });

  it("still brings the main window back when close-to-tray is switched off there", async () => {
    current.label = "main";
    await mount(false);
    expect(showMainWindow).toHaveBeenCalledTimes(1);
  });
});

// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const current = vi.hoisted(() => ({label: "main"}));
const win = vi.hoisted(() => ({
  closeHandler: undefined as ((e: {preventDefault: () => void}) => void) | undefined,
  hide: vi.fn(),
}));
const tray = vi.hoisted(() => ({showMainWindow: vi.fn(async () => {}), syncTray: vi.fn(), removeTray: vi.fn()}));
const settings = vi.hoisted(() => ({values: {} as Record<string, unknown>}));
const feedCalls = vi.hoisted(() => [] as unknown[]);

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: current.label,
    onCloseRequested: vi.fn(async (cb: (e: {preventDefault: () => void}) => void) => {
      win.closeHandler = cb;
      return () => {
        win.closeHandler = undefined;
      };
    }),
    hide: win.hide,
  }),
}));
vi.mock("@/lib/tray", () => tray);
// `isMainWindow` is stubbed from the same `current.label` handle the window mock reads, so the
// route-window case below still turns on the label it always did (#403 moved the comparison into
// `platform.ts`, and this factory mock replaces that module wholesale).
vi.mock("@/lib/platform", () => ({
  can: () => true,
  isMainWindow: async () => current.label === "main",
}));
vi.mock("@/lib/settings", () => ({
  useSetting: (key: string, fallback: unknown) => ({value: key in settings.values ? settings.values[key] : fallback}),
}));
vi.mock("@/lib/feed", () => ({
  useFeedStatus: (opts: unknown) => {
    feedCalls.push(opts);
    return {data: {pilots: 1204, healthy: true}};
  },
}));
vi.mock("@/lib/tmu", () => ({
  useTmis: () => ({data: [{status: "published"}, {status: "draft"}, {status: "cancelled"}, {status: "published"}]}),
  isActiveTmi: (t: {status: string}) => t.status === "published",
}));

import {CloseToTray, DesktopTray} from "./desktop-tray";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => {
  current.label = "main";
  win.closeHandler = undefined;
  win.hide.mockClear();
  settings.values = {};
  feedCalls.length = 0;
  for (const fn of Object.values(tray)) fn.mockClear();
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

async function render(node: React.ReactNode) {
  root = createRoot(document.createElement("div"));
  await act(async () => root!.render(node));
  await act(async () => new Promise((r) => setTimeout(r, 0)));
}

describe("CloseToTray (VATUSA/OIS#351 review)", () => {
  // It mounts in every non-embed window. Unguarded, a route window opening with close-to-tray off
  // (the default) called showMainWindow() and dragged focus back to the main window.
  it("leaves focus alone when a route window opens", async () => {
    current.label = "window--ops-idst";
    await render(<CloseToTray enabled={false} />);
    expect(tray.showMainWindow).not.toHaveBeenCalled();
  });

  it("still brings the main window back when close-to-tray is switched off there", async () => {
    await render(<CloseToTray enabled={false} />);
    expect(tray.showMainWindow).toHaveBeenCalledTimes(1);
  });

  it("turns closing the main window into hiding it", async () => {
    await render(<CloseToTray enabled />);
    const preventDefault = vi.fn();
    win.closeHandler?.({preventDefault});

    expect(preventDefault).toHaveBeenCalled();
    expect(win.hide).toHaveBeenCalled();
  });
});

describe("DesktopTray (VATUSA/OIS#351 review)", () => {
  // With the icon off there was nothing to bring a hidden window back or to quit from — on Windows
  // and Linux there is no dock either.
  it("does not hide to the tray while the tray icon is off", async () => {
    settings.values = {"tray.closeToTray": true, "tray.show": false};
    await render(<DesktopTray />);
    expect(win.closeHandler).toBeUndefined();
  });

  it("hides to the tray once the icon is on", async () => {
    settings.values = {"tray.closeToTray": true, "tray.show": true};
    await render(<DesktopTray />);
    expect(win.closeHandler).toBeDefined();
  });

  it("takes the icon away when the tray is off", async () => {
    await render(<DesktopTray />);
    expect(tray.removeTray).toHaveBeenCalled();
    expect(tray.syncTray).not.toHaveBeenCalled();
  });

  // The tray counted every TMI the list returned — drafts, expired, cancelled — while the dashboard
  // beside it counted live ones.
  it("shows the live TMI count the dashboard shows, not every TMI", async () => {
    settings.values = {"tray.show": true};
    await render(<DesktopTray />);
    expect(tray.syncTray).toHaveBeenLastCalledWith({pilots: 1204, activeTmis: 2, feedHealthy: true});
  });

  // Hidden to the tray, the document is hidden and TanStack skips interval polls — so the numbers
  // froze exactly when they were being read.
  it("keeps polling the feed while the window is hidden", async () => {
    settings.values = {"tray.show": true};
    await render(<DesktopTray />);
    expect(feedCalls).toContainEqual({background: true});
  });
});

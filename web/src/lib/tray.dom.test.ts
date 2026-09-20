// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {syncTray, trayStatusRows, trayTooltip} from "./tray";

const menuNew = vi.fn();
const trayNew = vi.fn();
const getById = vi.fn();

vi.mock("@tauri-apps/api/tray", () => ({
  TrayIcon: {
    new: (o: unknown) => trayNew(o),
    getById: (id: string) => getById(id),
    removeById: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/menu", () => ({Menu: {new: (o: unknown) => menuNew(o)}}));
const defaultWindowIcon = vi.fn();
vi.mock("@tauri-apps/api/app", () => ({defaultWindowIcon: () => defaultWindowIcon()}));

function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

const LIVE = {pilots: 1204, activeTmis: 3, feedHealthy: true};

beforeEach(() => {
  menuNew.mockReset().mockResolvedValue({});
  trayNew.mockReset().mockResolvedValue({});
  getById.mockReset().mockResolvedValue(null);
  defaultWindowIcon.mockReset().mockResolvedValue({rid: 1});
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("what the tray says", () => {
  it("summarises the live picture in one line", () => {
    expect(trayTooltip(LIVE)).toBe("OIS · 1,204 pilots · 3 TMIs · feed OK");
  });

  it("says so plainly when the feed is stale, rather than implying all is well", () => {
    expect(trayTooltip({...LIVE, feedHealthy: false})).toContain("feed stale");
  });

  it("shows a dash rather than a zero while a number is unknown", () => {
    // Reporting "0 pilots" when we simply haven't loaded yet would be a lie a controller might act on.
    expect(trayTooltip({})).toBe("OIS · — pilots · — TMIs · feed —");
    expect(trayStatusRows({})).toEqual([
      "Pilots online: —",
      "Active TMIs: —",
      "Feed: unknown",
    ]);
  });

  it("distinguishes a real zero from unknown", () => {
    expect(trayStatusRows({pilots: 0, activeTmis: 0, feedHealthy: true})).toEqual([
      "Pilots online: 0",
      "Active TMIs: 0",
      "Feed: healthy",
    ]);
  });
});

describe("syncTray", () => {
  it("does nothing on the web build", async () => {
    await expect(syncTray(LIVE)).resolves.toBe(false);
    expect(trayNew).not.toHaveBeenCalled();
  });

  it("builds the menu with the status as disabled rows, then actions", async () => {
    pretendDesktop();
    await expect(syncTray(LIVE)).resolves.toBe(true);

    const items = (menuNew.mock.calls[0]![0] as {items: {text?: string; enabled?: boolean}[]}).items;
    const statusRows = items.slice(0, 3);
    expect(statusRows.map((i) => i.text)).toEqual(trayStatusRows(LIVE));
    expect(statusRows.every((i) => i.enabled === false)).toBe(true);

    const labels = items.map((i) => i.text).filter(Boolean);
    expect(labels).toContain("Open OIS");
    expect(labels).toContain("TMU board");
    expect(labels).toContain("Facility map");
    expect(labels).toContain("Quit OIS");
  });

  it("updates the existing icon instead of adding a second one", async () => {
    // Re-syncing happens every time the numbers move; stacking icons would fill the menu bar.
    pretendDesktop();
    const existing = {setTooltip: vi.fn(), setMenu: vi.fn()};
    getById.mockResolvedValue(existing);

    await expect(syncTray(LIVE)).resolves.toBe(true);
    expect(trayNew).not.toHaveBeenCalled();
    expect(existing.setTooltip).toHaveBeenCalledWith(trayTooltip(LIVE));
    expect(existing.setMenu).toHaveBeenCalled();
  });

  it("reports failure rather than throwing when the tray can't be built", async () => {
    pretendDesktop();
    menuNew.mockRejectedValue(new Error("no menu bar"));

    await expect(syncTray(LIVE)).resolves.toBe(false);
  });
});

describe("when the tray cannot actually be seen", () => {
  it("refuses to create an icon-less tray, and says why", async () => {
    // This is the bug that shipped in the first cut: `core:app:allow-default-window-icon` is not
    // in Tauri's default permission set, so `defaultWindowIcon()` threw, the catch swallowed it,
    // and the menu bar simply stayed empty with no clue as to why. An invisible tray is worse
    // than an absent one, because the user has no way to tell them apart.
    pretendDesktop();
    defaultWindowIcon.mockResolvedValue(null);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    await expect(syncTray(LIVE)).resolves.toBe(false);
    expect(trayNew).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalled();

    warn.mockRestore();
  });

  it("reports the reason when the platform refuses outright", async () => {
    pretendDesktop();
    defaultWindowIcon.mockRejectedValue(new Error("app.default_window_icon not allowed"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    await expect(syncTray(LIVE)).resolves.toBe(false);
    expect(warn).toHaveBeenCalled();

    warn.mockRestore();
  });
});

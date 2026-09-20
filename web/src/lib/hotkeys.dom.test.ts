// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {applyHotkeys, hasModifier, normalizeAccelerator, runHotkey} from "./hotkeys";

const register = vi.fn();
const unregisterAll = vi.fn();
const openRouteWindow = vi.fn();
const showMainWindow = vi.fn();
const dismissAllAlerts = vi.fn();

vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: (a: string, cb: unknown) => register(a, cb),
  unregisterAll: () => unregisterAll(),
}));
vi.mock("@/lib/popout", () => ({openRouteWindow: (s: unknown) => openRouteWindow(s)}));
vi.mock("@/lib/tray", () => ({showMainWindow: () => showMainWindow()}));
vi.mock("@/lib/alerts", () => ({dismissAllAlerts: () => dismissAllAlerts()}));

function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

beforeEach(() => {
  register.mockReset().mockResolvedValue(undefined);
  unregisterAll.mockReset().mockResolvedValue(undefined);
  openRouteWindow.mockReset();
  showMainWindow.mockReset();
  dismissAllAlerts.mockReset();
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("normalizeAccelerator", () => {
  it("trims what the user typed", () => {
    expect(normalizeAccelerator("  CommandOrControl+Shift+O  ")).toBe("CommandOrControl+Shift+O");
  });

  it("treats blank and missing the same", () => {
    expect(normalizeAccelerator("   ")).toBe("");
    expect(normalizeAccelerator(undefined)).toBe("");
  });
});

describe("applyHotkeys", () => {
  it("registers nothing on the web build", async () => {
    await expect(applyHotkeys({focus: "CommandOrControl+Shift+O"})).resolves.toEqual([]);
    expect(register).not.toHaveBeenCalled();
  });

  it("releases the previous bindings before taking new ones", async () => {
    // Otherwise editing a shortcut leaves the old combination live — the bug you notice weeks later.
    pretendDesktop();
    await applyHotkeys({focus: "CommandOrControl+Shift+O"});

    expect(unregisterAll).toHaveBeenCalled();
    expect(register).toHaveBeenCalledWith("CommandOrControl+Shift+O", expect.any(Function));
  });

  it("registers nothing for an unset shortcut", async () => {
    // Empty is the default: installing an update must not take a key combination from the user.
    pretendDesktop();
    await expect(applyHotkeys({focus: "", tmu: "   "})).resolves.toEqual([]);
    expect(register).not.toHaveBeenCalled();
  });

  it("reports a combination the OS refuses instead of failing silently", async () => {
    pretendDesktop();
    register.mockRejectedValueOnce(new Error("already registered"));

    const results = await applyHotkeys({focus: "CommandOrControl+Space"});

    expect(results).toEqual([
      {
        action: "focus",
        accelerator: "CommandOrControl+Space",
        registered: false,
        reason: "unavailable",
      },
    ]);
  });

  it("keeps going after one shortcut is refused", async () => {
    // One conflict must not cost the user every other binding.
    pretendDesktop();
    register.mockRejectedValueOnce(new Error("already registered"));

    const results = await applyHotkeys({
      focus: "CommandOrControl+Space",
      tmu: "CommandOrControl+Shift+T",
    });

    expect(results.map((r) => r.registered)).toEqual([false, true]);
  });

  it("acts on press only, not on release as well", async () => {
    pretendDesktop();
    await applyHotkeys({focus: "CommandOrControl+Shift+O"});
    const handler = register.mock.calls[0]![1] as (e: {state: string}) => void;

    handler({state: "Pressed"});
    handler({state: "Released"});

    expect(showMainWindow).toHaveBeenCalledTimes(1);
  });
});

describe("runHotkey", () => {
  it("brings the app forward", () => {
    runHotkey("focus");
    expect(showMainWindow).toHaveBeenCalled();
  });

  it("clears the alerts on screen", () => {
    runHotkey("dismissAlerts");
    expect(dismissAllAlerts).toHaveBeenCalled();
  });

  it("opens the page a jump shortcut names", () => {
    runHotkey("tmu");
    expect(openRouteWindow).toHaveBeenCalledWith(expect.objectContaining({route: "/ops/tmu"}));
  });
});

describe("refusing a shortcut that would fire while you type", () => {
  it("recognises a real modifier combination", () => {
    expect(hasModifier("CommandOrControl+Shift+O")).toBe(true);
    expect(hasModifier("Alt+F1")).toBe(true);
    expect(hasModifier("  Control+K ")).toBe(true);
  });

  it("rejects a bare key", () => {
    // Bound globally, `O` fires every time you type the letter O in any application — including
    // the settings field you typed it into.
    expect(hasModifier("O")).toBe(false);
    expect(hasModifier("F5")).toBe(false);
    expect(hasModifier("")).toBe(false);
  });

  it("rejects something that only looks like a combination", () => {
    expect(hasModifier("A+B")).toBe(false);
  });

  it("refuses to register one, and says why", async () => {
    pretendDesktop();

    const results = await applyHotkeys({focus: "O"});

    expect(results).toEqual([
      {action: "focus", accelerator: "O", registered: false, reason: "no-modifier"},
    ]);
    expect(register).not.toHaveBeenCalled();
  });

  it("distinguishes that from a combination the OS already owns", async () => {
    pretendDesktop();
    register.mockRejectedValueOnce(new Error("already registered"));

    const [result] = await applyHotkeys({focus: "CommandOrControl+Space"});

    expect(result!.reason).toBe("unavailable");
  });
});

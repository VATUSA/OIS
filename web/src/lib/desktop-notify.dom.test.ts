// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";



const invoke = vi.fn();
const isPermissionGranted = vi.fn();
const requestPermission = vi.fn();
let clickHandler: ((event: {payload: unknown}) => void) | undefined;

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => isPermissionGranted(),
  requestPermission: () => requestPermission(),
}));
// The shell's own `notify` command raises the notification — the plugin reports no clicks on desktop.
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (event: string, handler: (e: {payload: unknown}) => void) => {
    if (event === "notification-clicked") clickHandler = handler;
    return () => {
      clickHandler = undefined;
    };
  },
}));

function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

const ALERT = {
  category: "restrictions",
  title: "Ground Stop: KATL",
  body: "Field-wide",
  route: "/ops/tmu?tab=ground-stops",
} as const;

/**
 * The OS permission answer is cached for the life of the module — deliberately, so the user is
 * asked once rather than per notification. Each case therefore needs its own copy of the module.
 */
async function freshModule() {
  vi.resetModules();
  return import("./desktop-notify");
}

beforeEach(() => {
  invoke.mockReset().mockResolvedValue(undefined);
  clickHandler = undefined;
  isPermissionGranted.mockReset().mockResolvedValue(true);
  requestPermission.mockReset().mockResolvedValue("granted");
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("notifyDesktop", () => {
  it("never fires on the web build, even when the setting is on", async () => {
    const {notifyDesktop} = await freshModule();
    await expect(notifyDesktop(ALERT, true)).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("never fires when the user hasn't opted that category in", async () => {
    pretendDesktop();
    const {notifyDesktop} = await freshModule();
    await expect(notifyDesktop(ALERT, false)).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("does not even ask the OS for permission when it isn't going to notify", async () => {
    // A permission prompt for a notification the user switched off would be indefensible.
    pretendDesktop();
    const {notifyDesktop} = await freshModule();
    await notifyDesktop(ALERT, false);
    expect(isPermissionGranted).not.toHaveBeenCalled();
    expect(requestPermission).not.toHaveBeenCalled();
  });

  it("fires on desktop when opted in, carrying the route for the click to follow", async () => {
    pretendDesktop();
    const {notifyDesktop} = await freshModule();
    await expect(notifyDesktop(ALERT, true)).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("notify", {
      title: "Ground Stop: KATL",
      body: "Field-wide",
      route: "/ops/tmu?tab=ground-stops",
    });
  });

  it("stays silent when the OS refuses permission", async () => {
    pretendDesktop();
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("denied");
    const {notifyDesktop} = await freshModule();

    await expect(notifyDesktop(ALERT, true)).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reports failure rather than throwing when the shell errors", async () => {
    // A notification failing must never take down the surface that triggered it.
    pretendDesktop();
    invoke.mockRejectedValue(new Error("notification centre unavailable"));
    const {notifyDesktop} = await freshModule();

    await expect(notifyDesktop(ALERT, true)).resolves.toBe(false);
  });
});

describe("listenForNotificationClicks", () => {
  it("hands a clicked notification's route to navigate", async () => {
    pretendDesktop();
    const {listenForNotificationClicks} = await freshModule();
    const navigate = vi.fn();

    const dispose = await listenForNotificationClicks(navigate);
    clickHandler?.({payload: "/ops/tmu?tab=gdp"});

    expect(navigate).toHaveBeenCalledWith("/ops/tmu?tab=gdp");
    dispose?.();
    expect(clickHandler).toBeUndefined();
  });

  it("listens for nothing on the web build", async () => {
    const {listenForNotificationClicks} = await freshModule();
    await expect(listenForNotificationClicks(vi.fn())).resolves.toBeUndefined();
    expect(clickHandler).toBeUndefined();
  });
});

// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";



const sendNotification = vi.fn();
const isPermissionGranted = vi.fn();
const requestPermission = vi.fn();

vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: (...a: unknown[]) => sendNotification(...a),
  isPermissionGranted: () => isPermissionGranted(),
  requestPermission: () => requestPermission(),
  onAction: vi.fn(),
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
  sendNotification.mockReset();
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
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("never fires when the user hasn't opted that category in", async () => {
    pretendDesktop();
    const {notifyDesktop} = await freshModule();
    await expect(notifyDesktop(ALERT, false)).resolves.toBe(false);
    expect(sendNotification).not.toHaveBeenCalled();
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
    expect(sendNotification).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Ground Stop: KATL",
        body: "Field-wide",
        extra: {"ois.route": "/ops/tmu?tab=ground-stops"},
      }),
    );
  });

  it("stays silent when the OS refuses permission", async () => {
    pretendDesktop();
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("denied");
    const {notifyDesktop} = await freshModule();

    await expect(notifyDesktop(ALERT, true)).resolves.toBe(false);
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("reports failure rather than throwing when the plugin errors", async () => {
    // A notification failing must never take down the surface that triggered it.
    pretendDesktop();
    sendNotification.mockImplementation(() => {
      throw new Error("notification centre unavailable");
    });
    const {notifyDesktop} = await freshModule();

    await expect(notifyDesktop(ALERT, true)).resolves.toBe(false);
  });
});

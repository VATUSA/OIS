// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

const post = vi.fn();
vi.mock("@/lib/api", () => ({ API_BASE: "http://127.0.0.1:3000", ois: { POST: (...a: unknown[]) => post(...a) } }));
vi.mock("@/lib/desktop-token", () => ({
  getDesktopToken: async () => "ois_dsk_stored",
  setDesktopToken: vi.fn(),
}));
const platform = vi.hoisted(() => ({tauri: true, main: true}));
vi.mock("@/lib/platform", () => ({
  isTauri: () => platform.tauri,
  isMainWindow: async () => platform.main,
  invokeDesktop: vi.fn(async () => undefined),
}));

import {LAUNCH_REFRESH_BUDGET_MS, refreshBeforeLaunch, rotateOnLaunch} from "./desktop-auth";

afterEach(() => {
  vi.useRealTimers();
  post.mockReset();
  platform.tauri = true;
  platform.main = true;
});

describe("refreshBeforeLaunch (VATUSA/OIS#346)", () => {
  // A blackholed API host leaves the request pending until the OS TCP timeout. Launch must not wait
  // that long on a blank window.
  it("lets the app render once the budget passes, even if the API never answers", async () => {
    vi.useFakeTimers();
    post.mockReturnValue(new Promise(() => {}));
    let launched = false;
    void refreshBeforeLaunch().then(() => (launched = true));

    await vi.advanceTimersByTimeAsync(LAUNCH_REFRESH_BUDGET_MS - 1);
    expect(launched).toBe(false); // the rotation still gets its chance first
    await vi.advanceTimersByTimeAsync(1);
    expect(launched).toBe(true);
  });

  it("never rejects, so a failed rotation cannot stop the app rendering", async () => {
    post.mockRejectedValue(new Error("network down"));
    await expect(refreshBeforeLaunch()).resolves.toBeUndefined();
  });
});

describe("rotateOnLaunch (VATUSA/OIS#349 review)", () => {
  const rotated = () => post.mock.calls.some(([path]) => path === "/api/v1/auth/desktop/refresh");

  it("rotates the session in the main window", async () => {
    post.mockResolvedValue({data: {token: "ois_dsk_new"}});
    await rotateOnLaunch();
    expect(rotated()).toBe(true);
  });

  // Every window loads the same entry. A pop-out rotating deleted the token the main window still
  // held, so popping a panel out signed the user out.
  it("leaves the session alone in a pop-out", async () => {
    platform.main = false;
    await rotateOnLaunch();
    expect(post).not.toHaveBeenCalled();
  });

  it("does nothing on the web build", async () => {
    platform.tauri = false;
    await rotateOnLaunch();
    expect(post).not.toHaveBeenCalled();
  });
});

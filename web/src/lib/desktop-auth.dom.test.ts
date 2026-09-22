// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

const post = vi.fn();
vi.mock("@/lib/api", () => ({ API_BASE: "http://127.0.0.1:3000", ois: { POST: (...a: unknown[]) => post(...a) } }));
vi.mock("@/lib/desktop-token", () => ({
  getDesktopToken: async () => "ois_dsk_stored",
  setDesktopToken: vi.fn(),
}));
vi.mock("@/lib/platform", () => ({ isTauri: () => true, invokeDesktop: vi.fn(async () => undefined) }));

import {LAUNCH_REFRESH_BUDGET_MS, refreshBeforeLaunch} from "./desktop-auth";

afterEach(() => {
  vi.useRealTimers();
  post.mockReset();
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

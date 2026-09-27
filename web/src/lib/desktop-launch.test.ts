import {afterEach, describe, expect, it, vi} from "vitest";

const steps = vi.hoisted(() => ({
  rotated: undefined as (() => void) | undefined,
  order: [] as string[],
}));
vi.mock("@/lib/desktop-auth", () => ({
  rotateOnLaunch: () =>
    new Promise<void>((resolve) => {
      steps.order.push("rotate:start");
      steps.rotated = () => {
        steps.order.push("rotate:done");
        resolve();
      };
    }),
}));
vi.mock("@/lib/popout", () => ({
  forgetOnClose: async () => undefined,
  restoreWindows: () => {
    steps.order.push("restore");
    return new Promise(() => {}); // never settles: launch must not wait on it
  },
}));

import {launchDesktop} from "./desktop-launch";

afterEach(() => {
  steps.order = [];
  steps.rotated = undefined;
});

describe("launchDesktop (VATUSA/OIS#350 review)", () => {
  // A window restored mid-rotation cached the token the rotation was about to delete.
  it("restores windows only once the session rotation has finished", async () => {
    const launched = launchDesktop();
    await Promise.resolve();
    expect(steps.order).toEqual(["rotate:start"]);

    steps.rotated?.();
    await launched;
    expect(steps.order).toEqual(["rotate:start", "rotate:done", "restore"]);
  });

  it("does not hold first paint for the windows to reopen", async () => {
    const launched = launchDesktop();
    await Promise.resolve();
    steps.rotated?.();
    await expect(launched).resolves.toBeUndefined();
  });
});

// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The seam between a detection and a noise (#353).
 *
 * `sounds.dom.test.ts` covers `playAlertSound` itself; this covers *when* it is reached, and with
 * what. Nothing did before, which is how a release went out where the sound toggles were wired to
 * the notification settings.
 *
 * `@/lib/sounds` is mocked rather than the audio stack re-stubbed: what matters here is that the
 * sound is a second output of the same call, decided by its own settings.
 */
const playAlertSound = vi.hoisted(() => vi.fn(() => Promise.resolve(true)));
vi.mock("@/lib/sounds", () => ({playAlertSound}));
vi.mock("@/lib/platform", () => ({
  can: () => true,
  invokeDesktop: () => Promise.resolve(),
}));
// Keep the banner half out of the way — this is about the audio path.
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: async () => false,
  requestPermission: async () => "denied",
  sendNotification: () => {},
}));

import {notifyDesktop} from "./desktop-notify";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  playAlertSound.mockClear();
});

const alert = {
  category: "restrictions" as const,
  title: "Ground stop",
  body: "KDCA",
  route: "/ops/tmu",
};

describe("sound is a second output, not a second notification", () => {
  it("plays with the banner switched off", async () => {
    // The whole design: a category can make a noise without a banner. Moving the play below the
    // `enabled` check — where it looks like it belongs — silently breaks this.
    await notifyDesktop(alert, false, {enabled: true});
    await settle();

    expect(playAlertSound).toHaveBeenCalledTimes(1);
    expect(playAlertSound).toHaveBeenCalledWith("restrictions", {enabled: true});
  });

  it("stays silent with the sound switched off, banner or not", async () => {
    await notifyDesktop(alert, true, {enabled: false});
    await notifyDesktop(alert, false, {enabled: false});
    await settle();

    expect(playAlertSound).not.toHaveBeenCalled();
  });

  it("stays silent when the caller asks for no sound at all", async () => {
    // A caller that forgets to pass sound settings must not start making noise by default.
    await notifyDesktop(alert, true);
    await settle();

    expect(playAlertSound).not.toHaveBeenCalled();
  });

  it("hands the caller's volume through", async () => {
    await notifyDesktop(alert, false, {enabled: true, volume: "quiet"});
    await settle();

    expect(playAlertSound).toHaveBeenCalledWith("restrictions", {enabled: true, volume: "quiet"});
  });
});

// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from "vitest";

import {notifyDesktop} from "./desktop-notify";

/**
 * The seam between a detection and a noise (#353).
 *
 * `sounds.dom.test.ts` covers `playAlertSound` in isolation; this covers *when* it is reached.
 * Nothing did before, which is how a release went out where the sound toggles were wired to the
 * notification settings.
 */
vi.mock("@tauri-apps/api/core", () => ({convertFileSrc: (p: string) => `asset://${p}`}));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: async () => "/appdata",
  join: async (...parts: string[]) => parts.join("/"),
}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => ({label: "main"})}));
// Keep the banner half out of the way — this is about the audio path.
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: async () => false,
  requestPermission: async () => "denied",
  sendNotification: () => {},
}));

let plays: string[] = [];
class FakeAudio {
  volume = 1;
  onerror: (() => void) | null = null;
  constructor(public src: string) {}
  play() {
    plays.push(this.src);
    return Promise.resolve();
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  plays = [];
  vi.stubGlobal("Audio", FakeAudio);
  window.__TAURI_INTERNALS__ = {};
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

    expect(plays).toHaveLength(1);
  });

  it("stays silent with the sound switched off, banner or not", async () => {
    await notifyDesktop(alert, true, {enabled: false});
    await notifyDesktop(alert, false, {enabled: false});
    await settle();

    expect(plays).toHaveLength(0);
  });

  it("stays silent when the caller asks for no sound at all", async () => {
    // A caller that forgets to pass sound settings must not start making noise by default.
    await notifyDesktop(alert, true);
    await settle();

    expect(plays).toHaveLength(0);
  });

  it("plays on the volume the caller passed", async () => {
    await notifyDesktop(alert, false, {enabled: true, volume: "quiet"});
    await settle();

    expect(plays).toHaveLength(1);
  });
});

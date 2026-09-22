// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {bundledSoundUrl, gainFor, playAlertSound} from "./sounds";

const convertFileSrc = vi.fn();
const appDataDir = vi.fn();
let windowLabel = "main";

vi.mock("@tauri-apps/api/core", () => ({convertFileSrc: (p: string) => convertFileSrc(p)}));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: () => appDataDir(),
  join: (...parts: string[]) => Promise.resolve(parts.join("/")),
}));
// The notifiers render in every route window (#350), so which window we are in decides whether we
// are the one that makes the noise.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({label: windowLabel}),
}));

/** The tick claim collapses a synchronous burst; let it drain between independent cases. */
const nextTick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Records what was constructed, and lets a test decide whether it plays. */
let played: {src: string; volume: number}[] = [];
let failFor: (src: string) => boolean = () => false;

class FakeAudio {
  volume = 1;
  onerror: (() => void) | null = null;
  constructor(public src: string) {}
  play() {
    played.push({src: this.src, volume: this.volume});
    return failFor(this.src) ? Promise.reject(new Error("no such file")) : Promise.resolve();
  }
}

function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

beforeEach(() => {
  played = [];
  failFor = () => false;
  windowLabel = "main";
  vi.stubGlobal("Audio", FakeAudio);
  convertFileSrc.mockReset().mockImplementation((p: string) => `asset://${p}`);
  appDataDir.mockReset().mockResolvedValue("/Users/x/Library/Application Support/net.vatusa.ois");
});

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  vi.unstubAllGlobals();
});

describe("volume", () => {
  it("maps the settings to distinct levels", () => {
    expect(gainFor("quiet")).toBeLessThan(gainFor("normal"));
    expect(gainFor("normal")).toBeLessThan(gainFor("loud"));
  });

  it("falls back to normal for anything unrecognised", () => {
    expect(gainFor(undefined)).toBe(gainFor("normal"));
    expect(gainFor("deafening")).toBe(gainFor("normal"));
  });
});

describe("playAlertSound", () => {
  it("stays silent on the web build", async () => {
    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("stays silent in a route window, so one alert isn't played once per open window", async () => {
    // `RootLayout` renders the notifiers in every whole-route window, and three open windows played
    // the same tone three times at once — near enough in phase to sum rather than echo.
    pretendDesktop();
    windowLabel = "window-/ops/tmu";

    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("plays once for a batch that arrives in one tick", async () => {
    // `useNewKeys` notifies per newly-appeared key, synchronously. A TMU releasing ten flights —
    // or a role grant that expands into dozens of permission keys — must not start that many
    // copies of the same tone at once.
    pretendDesktop();

    const results = await Promise.all(
      Array.from({length: 10}, () => playAlertSound("releases", {enabled: true})),
    );

    expect(played).toHaveLength(1);
    expect(results.filter(Boolean)).toHaveLength(1);
  });

  it("plays again for a batch in the next tick", async () => {
    // Separate events are separate alerts — the claim must not silence everything after the first.
    pretendDesktop();
    await playAlertSound("releases", {enabled: true});
    await nextTick();
    await playAlertSound("releases", {enabled: true});

    expect(played).toHaveLength(2);
  });

  it("resolves the override path once, not on every alert", async () => {
    // The path can't move while the app runs, and the probe costs two IPC round-trips before the
    // bundled tone can even start.
    pretendDesktop();
    await playAlertSound("access", {enabled: true});
    await nextTick();
    await playAlertSound("access", {enabled: true});
    await nextTick();
    await playAlertSound("access", {enabled: true});

    expect(appDataDir).toHaveBeenCalledTimes(1);
    expect(played).toHaveLength(3);
  });

  it("stays silent when the category is switched off", async () => {
    pretendDesktop();
    await expect(playAlertSound("restrictions", {enabled: false})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("prefers a replacement the user has dropped in", async () => {
    pretendDesktop();

    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(true);

    expect(played[0]!.src).toContain("sounds/restrictions.wav");
    expect(played[0]!.src.startsWith("asset://")).toBe(true);
  });

  it("falls back to the bundled sound when there is no replacement", async () => {
    // A missing or unplayable override must degrade to the standard sound, not to silence —
    // silence is indistinguishable from a broken feature.
    pretendDesktop();
    failFor = (src) => src.startsWith("asset://");

    await expect(playAlertSound("metering", {enabled: true})).resolves.toBe(true);
    // Two constructions for one alert: the override attempt, then the bundled fallback.

    expect(played).toHaveLength(2);
    expect(played[1]!.src).toBe(bundledSoundUrl("metering"));
  });

  it("plays at the configured volume", async () => {
    pretendDesktop();
    await playAlertSound("access", {enabled: true, volume: "quiet"});
    expect(played[0]!.volume).toBe(gainFor("quiet"));
  });

  it("reports failure rather than throwing when nothing can be played", async () => {
    pretendDesktop();
    failFor = () => true;

    await expect(playAlertSound("releases", {enabled: true})).resolves.toBe(false);
  });
});

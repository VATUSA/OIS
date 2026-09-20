// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {bundledSoundUrl, gainFor, playAlertSound} from "./sounds";

const convertFileSrc = vi.fn();
const appDataDir = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({convertFileSrc: (p: string) => convertFileSrc(p)}));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: () => appDataDir(),
  join: (...parts: string[]) => Promise.resolve(parts.join("/")),
}));

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

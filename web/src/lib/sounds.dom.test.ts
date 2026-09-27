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

/** What played, and which URLs were fetched how. */
let played: {src: string; volume: number}[] = [];
let fetches: {url: string; init?: RequestInit}[] = [];
let failFor: (src: string) => boolean = () => false;

/**
 * Web Audio, faked end to end: `fetch` hands back a buffer tagged with its URL, the context decodes
 * it, and starting a source records what played and at what gain.
 */
type Tagged = {src: string};
function installFakeAudio(record: (p: {src: string; volume: number}) => void, fails: (src: string) => boolean) {
  vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
    fetches.push({url, init});
    return fails(url)
      ? {ok: false, arrayBuffer: async () => ({src: url})}
      : {ok: true, arrayBuffer: async () => ({src: url})};
  });
  class FakeContext {
    state = "running";
    destination = {};
    async resume() {}
    async decodeAudioData(data: Tagged) {
      return {src: data.src};
    }
    createGain() {
      const node = {gain: {value: 1}, connect: (next: unknown) => next};
      return node;
    }
    createBufferSource() {
      let level: {gain: {value: number}} | undefined;
      const source = {
        buffer: undefined as Tagged | undefined,
        connect(next: {gain: {value: number}}) {
          level = next;
          return next;
        },
        start() {
          record({src: source.buffer!.src, volume: level?.gain.value ?? 1});
        },
      };
      return source;
    }
  }
  vi.stubGlobal("AudioContext", FakeContext);
}

/**
 * A desktop environment complete enough for the *real* `@tauri-apps/api/window` too, not just the
 * mock: the real module reads the window label from `__TAURI_INTERNALS__.metadata`, and with a bare
 * `{}` it threw, `isMainWindow` took its "can't tell, stay quiet" path, and the burst test passed
 * whether or not the per-tick claim existed (VATUSA/OIS#353 review).
 */
function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {
    metadata: {
      get currentWindow() {
        return {label: windowLabel};
      },
      get currentWebview() {
        return {label: windowLabel, windowLabel};
      },
    },
  };
}

beforeEach(() => {
  played = [];
  fetches = [];
  failFor = () => false;
  windowLabel = "main";
  installFakeAudio((p) => played.push(p), (src) => failFor(src));
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

  // Settings are a free-form blob; an inherited key came back as a function and silenced the category.
  it("ignores keys that only exist on the object prototype", () => {
    expect(gainFor("constructor")).toBe(gainFor("normal"));
    expect(gainFor("toString")).toBe(gainFor("normal"));
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
    // The override was tried and failed; the bundled tone is what played.
    expect(fetches.map((f) => f.url)).toEqual([expect.stringContaining("asset://"), bundledSoundUrl("metering")]);
    expect(played).toEqual([{src: bundledSoundUrl("metering"), volume: gainFor("normal")}]);
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

  // A media element streams with Range requests, which the release build's `tauri://` protocol
  // doesn't answer; a whole-file GET works over every scheme. `no-store` so a replaced file plays.
  it("fetches the whole file, uncached, rather than streaming it", async () => {
    pretendDesktop();
    await playAlertSound("restrictions", {enabled: true});
    expect(fetches[0]!.init).toMatchObject({cache: "no-store"});
    expect(fetches[0]!.init?.headers).toBeUndefined();
  });

  // A transient failure resolving the override path used to be cached as "no override" for good.
  it("tries the override again after a failed resolution", async () => {
    pretendDesktop();
    appDataDir.mockRejectedValueOnce(new Error("ipc not ready"));
    await playAlertSound("eventReminders", {enabled: true});
    await nextTick();
    await playAlertSound("eventReminders", {enabled: true});

    expect(played.map((p) => p.src)).toEqual([
      bundledSoundUrl("eventReminders"),
      expect.stringContaining("asset://"),
    ]);
  });
});

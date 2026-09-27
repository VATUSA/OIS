// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from "vitest";

type Sounds = typeof import("./sounds");
let bundledSoundUrl: Sounds["bundledSoundUrl"];
let previewAlertSound: Sounds["previewAlertSound"];
let gainFor: Sounds["gainFor"];
let playAlertSound: Sounds["playAlertSound"];

const platform = vi.hoisted(() => ({audioAlerts: true, main: true}));
// Mocked as a module rather than through `window.__TAURI_INTERNALS__`: `isMainWindow` used to be a
// local copy here reached by a dynamic `import()`, and ten concurrent calls to a *mocked* dynamic
// import fail for all but the first — so a burst looked collapsed whether or not the per-tick claim
// existed, and the test that was meant to pin the claim passed without it (VATUSA/OIS#353 review).
vi.mock("@/lib/platform", () => ({
  can: () => platform.audioAlerts,
  isMainWindow: () => Promise.resolve(platform.main),
}));

const appDataDir = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({convertFileSrc: (p: string) => `asset://${p}`}));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: () => appDataDir(),
  join: (...parts: string[]) => Promise.resolve(parts.join("/")),
}));

/** Which URLs a fetch should refuse, so a test can say "there is no replacement". */
let missing: (url: string) => boolean = () => false;
let fetched: string[] = [];
let fetchInits: (RequestInit | undefined)[] = [];
/** The contents of each file on "disk", as a version number, so a test can overwrite one. */
let onDisk: Record<string, number> = {};
/** Every tone that actually reached the output, with the gain it was played at and the file version. */
let played: {src: string; gain: number; version?: number}[] = [];
let resumed = 0;

class FakeGain {
  gain = {value: 1};
  connect() {}
}
class FakeSource {
  buffer: {url: string; version?: number} | null = null;
  private out: FakeGain | undefined;
  connect(node: FakeGain) {
    this.out = node;
  }
  start() {
    played.push({src: this.buffer?.url ?? "?", gain: this.out?.gain.value ?? -1, version: this.buffer?.version});
  }
}
class FakeAudioContext {
  state = "suspended";
  destination = {};
  resume() {
    resumed += 1;
    this.state = "running";
    return Promise.resolve();
  }
  createBufferSource() {
    return new FakeSource();
  }
  createGain() {
    return new FakeGain();
  }
  // The decoded "buffer" just remembers which URL it came from, which is what tests assert on.
  decodeAudioData(bytes: ArrayBuffer & {url?: string; version?: number}) {
    return Promise.resolve({url: bytes.url ?? "?", version: bytes.version});
  }
}

const drain = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(async () => {
  played = [];
  fetched = [];
  fetchInits = [];
  onDisk = {};
  resumed = 0;
  missing = () => false;
  platform.audioAlerts = true;
  platform.main = true;
  appDataDir.mockReset().mockResolvedValue("/Users/x/Library/Application Support/net.vatusa.ois");
  vi.stubGlobal("AudioContext", FakeAudioContext);
  vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
    fetched.push(url);
    fetchInits.push(init);
    if (missing(url)) return Promise.resolve({ok: false, arrayBuffer: () => Promise.resolve({})});
    const bytes = {url, version: onDisk[url] ?? 1} as unknown as ArrayBuffer;
    return Promise.resolve({ok: true, arrayBuffer: () => Promise.resolve(bytes)});
  });
  // Decoded tones, resolved override paths and the AudioContext all live for the process by
  // design, so each case takes a fresh module rather than inheriting the last one's caches.
  vi.resetModules();
  ({bundledSoundUrl, gainFor, playAlertSound, previewAlertSound} = await import("./sounds"));
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

  it("falls back for a prototype key, rather than handing back a function", () => {
    // Settings are a free-form blob, so these can genuinely arrive. Indexing the prototype returned
    // a function, and assigning it as a gain threw — silencing the category (#353 review).
    expect(gainFor("constructor")).toBe(gainFor("normal"));
    expect(gainFor("toString")).toBe(gainFor("normal"));
    expect(gainFor("__proto__")).toBe(gainFor("normal"));
  });
});

describe("playAlertSound", () => {
  it("stays silent when the platform has no audio alerts", async () => {
    platform.audioAlerts = false;
    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("stays silent when the category is switched off", async () => {
    await expect(playAlertSound("restrictions", {enabled: false})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("stays silent in a route window, so one alert isn't played once per open window", async () => {
    platform.main = false;
    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });

  it("plays once for a burst that arrives in one tick", async () => {
    // `useNewKeys` notifies per newly-appeared key, synchronously. Ten new releases must not start
    // ten copies of the same tone at once; summed, they clip and read as a blast.
    for (let i = 0; i < 10; i++) void playAlertSound("releases", {enabled: true});
    await drain();

    expect(played).toHaveLength(1);
  });

  it("plays again for a burst in the next tick", async () => {
    // Separate events are separate alerts — the claim must not silence everything after the first.
    await playAlertSound("releases", {enabled: true});
    await drain();
    await playAlertSound("releases", {enabled: true});
    await drain();

    expect(played).toHaveLength(2);
  });

  it("prefers a replacement the user has dropped in", async () => {
    await playAlertSound("restrictions", {enabled: true});
    await drain();

    expect(played).toHaveLength(1);
    expect(played[0]!.src).toContain("asset://");
    expect(played[0]!.src).toContain("sounds/restrictions.wav");
  });

  it("falls back to the bundled tone when there is no replacement", async () => {
    missing = (url) => url.startsWith("asset://");
    await playAlertSound("metering", {enabled: true});
    await drain();

    expect(played).toHaveLength(1);
    expect(played[0]!.src).toBe(bundledSoundUrl("metering"));
  });

  it("plays at the configured volume", async () => {
    await playAlertSound("access", {enabled: true, volume: "quiet"});
    await drain();

    expect(played[0]!.gain).toBe(gainFor("quiet"));
  });

  it("resumes a suspended context, or the first alert of a session is silent", async () => {
    await playAlertSound("access", {enabled: true});
    await drain();

    expect(resumed).toBe(1);
    expect(played).toHaveLength(1);
  });

  it("decodes each tone once, not on every alert", async () => {
    missing = (url) => url.startsWith("asset://");
    for (const _ of [1, 2, 3]) {
      await playAlertSound("releases", {enabled: true});
      await drain();
    }

    expect(played).toHaveLength(3);
    // Three alerts, one fetch of the bundled tone. The override 404s, and a failure is deliberately
    // not cached, so it is retried each time.
    expect(fetched.filter((u) => u === bundledSoundUrl("releases"))).toHaveLength(1);
  });

  it("retries a replacement that was briefly unreadable", async () => {
    missing = (url) => url.startsWith("asset://");
    await playAlertSound("eventReminders", {enabled: true});
    await drain();
    expect(played[0]!.src).toBe(bundledSoundUrl("eventReminders"));

    // The file becomes readable — caching the earlier failure would have written it off until the
    // app restarted (#353 review).
    missing = () => false;
    await playAlertSound("eventReminders", {enabled: true});
    await drain();

    expect(played[1]!.src).toContain("asset://");
  });

  it("reports failure rather than throwing when nothing can be played", async () => {
    missing = () => true;
    await expect(playAlertSound("restrictions", {enabled: true})).resolves.toBe(false);
    expect(played).toHaveLength(0);
  });
});

/**
 * The settings preview (#404).
 *
 * It exists because the fallback to the bundled tone is silent, so a replacement that can't be read
 * looked exactly like one that works. It shares `playAlertSound`'s resolution but deliberately not its
 * guards — each of those would be wrong for a button the user just pressed.
 */
describe("previewAlertSound", () => {
  it("reports the bundled default when the replacement resolves but won't play", async () => {
    // The case a user actually hits: a file is named, and it is unreadable or a codec the webview
    // rejects. Without this answer the fallback is indistinguishable from success.
    missing = (url) => url.startsWith("asset://");

    await expect(previewAlertSound("restrictions")).resolves.toBe("bundled");
    expect(played[0]!.src).toBe(bundledSoundUrl("restrictions"));
  });

  it("reports the user's own file when it plays", async () => {
    await expect(previewAlertSound("releases")).resolves.toBe("replacement");
    expect(played[0]!.src).toContain("asset://");
  });

  it("reports failure when nothing can be played", async () => {
    missing = () => true;

    await expect(previewAlertSound("metering")).resolves.toBe("none");
    expect(played).toHaveLength(0);
  });

  it("stays silent on a build without audio alerts", async () => {
    platform.audioAlerts = false;

    await expect(previewAlertSound("access")).resolves.toBe("none");
    expect(played).toHaveLength(0);
  });

  it("sounds again on a second press in the same tick", async () => {
    // `playAlertSound`'s per-tick claim collapses a burst of detections into one sound. Pressing a
    // button twice is two requests, not a burst, so the claim must not apply here.
    //
    // Warmed first, and deliberately: `overrideSoundUrl` resolves the path through a *mocked*
    // dynamic `import()`, and ten concurrent calls to one of those fail for all but the first — the
    // same harness artifact that let #353 ship a burst test which passed without the claim. One
    // awaited call fills the path cache so the pair below measures the claim and nothing else.
    await previewAlertSound("releases");
    played = [];

    const [first, second] = await Promise.all([
      previewAlertSound("releases"),
      previewAlertSound("releases"),
    ]);

    expect([first, second]).toEqual(["replacement", "replacement"]);
    expect(played).toHaveLength(2);
  });

  it("plays from a route window, where the settings page can be open", async () => {
    // An alert only sounds in the main window, or one ground stop would play once per open window
    // (#350). A preview is asked for by the window it is pressed in.
    platform.main = false;

    await expect(previewAlertSound("eventReminders")).resolves.toBe("replacement");
    expect(played).toHaveLength(1);
  });

  it("plays at the volume the row is set to", async () => {
    await previewAlertSound("access", "loud");

    expect(played[0]!.gain).toBe(gainFor("loud"));
  });

  // "Did my change take?" is the question the button answers. Decoded tones are kept for the process,
  // so an overwritten replacement replayed its stale decoded copy while still saying "Your file".
  it("plays the replacement as it is on disk now, after it has been overwritten", async () => {
    await playAlertSound("restrictions", {enabled: true});
    const override = played[0]!.src;
    expect(played[0]!.version).toBe(1);
    await drain();

    onDisk[override] = 2; // the facility overwrites its tone while the app runs
    played = [];
    await expect(previewAlertSound("restrictions")).resolves.toBe("replacement");

    expect(played[0]!.version).toBe(2);
  });

  it("lets later alerts in the window use the file the preview just read", async () => {
    await playAlertSound("releases", {enabled: true});
    const override = played[0]!.src;
    await drain();
    onDisk[override] = 2;

    await previewAlertSound("releases");
    await drain();
    played = [];
    await playAlertSound("releases", {enabled: true});

    expect(played[0]!.version).toBe(2);
  });

  it("fetches around the webview's HTTP cache, which would keep an overwritten file's old bytes", async () => {
    await previewAlertSound("access");
    expect(fetchInits[0]).toMatchObject({cache: "no-store"});
  });
});

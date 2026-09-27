import type {NotifyCategory} from "@/lib/desktop-notify";
import {can, isMainWindow} from "@/lib/platform";

/**
 * Audible alerts for the desktop app (#353).
 *
 * The point of doing this natively: a browser tab throttles timers and gates autoplay, so a web app
 * cannot be relied on to make a noise when a ground stop lands. A desktop app can.
 *
 * Sound is a second output from #348's detection, not a second detector — `notifyDesktop` is where
 * every category already converges, so a notification and its sound can never disagree about what
 * happened.
 *
 * Each sound ships as a default but is **replaceable**: drop a `.wav` of the same name into the
 * OIS app-data folder and it is used instead, so a facility can use its own tones without a build.
 *
 * Played through **Web Audio** rather than an `Audio` element. In a release build the bundle is
 * served over Tauri's `tauri://` protocol, which has no byte-range handling, and a webview's media
 * stack asks for media with Range requests — so `<audio>` could fail in exactly the build that
 * ships, while working in dev over Vite's http. A plain `fetch` needs no Range support, so dev and
 * release take the same path (VATUSA/OIS#353 review).
 */

/** How loud, as the settings offer it. */
export type AlertVolume = "quiet" | "normal" | "loud";

const GAIN: Record<AlertVolume, number> = {
  quiet: 0.25,
  normal: 0.6,
  loud: 1,
};

export function gainFor(volume: string | undefined): number {
  const key = volume ?? "normal";
  // Own properties only. Settings are a free-form blob, so `"constructor"` would otherwise index
  // the prototype, hand back a function, and throw when assigned as a gain — silencing that
  // category (VATUSA/OIS#353 review).
  return Object.hasOwn(GAIN, key) ? GAIN[key as AlertVolume] : GAIN.normal;
}

/** The sound bundled with the app, served like any other public asset. */
export function bundledSoundUrl(category: NotifyCategory): string {
  return `/sounds/${category}.wav`;
}

/**
 * A user-supplied replacement, if one exists.
 *
 * Resolved with `convertFileSrc`, which hands the webview a URL for a file on disk — so no
 * filesystem plugin and no read permissions are needed. Whether the file *exists* is answered by
 * trying to fetch it and falling back, rather than by asking the filesystem.
 */
async function resolveOverrideUrl(category: NotifyCategory): Promise<string> {
  const {appDataDir, join} = await import("@tauri-apps/api/path");
  const {convertFileSrc} = await import("@tauri-apps/api/core");
  return convertFileSrc(await join(await appDataDir(), "sounds", `${category}.wav`));
}

/**
 * Resolved override URLs, cached for the life of the process.
 *
 * The path does not move while the app is running, and almost nobody has dropped a file in — so
 * without this every alert paid for two IPC round-trips before it could fall back to the bundled
 * tone. A *failure* is deliberately not cached: caching it meant one transient IPC error disabled a
 * facility's replacement tone until the app restarted (VATUSA/OIS#353 review).
 */
const overrideUrls = new Map<NotifyCategory, string>();

async function overrideSoundUrl(category: NotifyCategory): Promise<string | undefined> {
  const known = overrideUrls.get(category);
  if (known) return known;
  try {
    const url = await resolveOverrideUrl(category);
    overrideUrls.set(category, url);
    return url;
  } catch {
    return undefined;
  }
}

/**
 * Categories that have already claimed a sound in the current tick.
 *
 * `useNewKeys` calls `notifyDesktop` once per newly-appeared key, synchronously, so a batch — a TMU
 * releasing ten flights, or a role grant that expands into dozens of permission keys — would start
 * that many copies of one 0.38s tone at once. Summed amplitudes clip and it reads as a blast rather
 * than an alert. One sound per category per batch; the set is emptied on the next microtask, so
 * genuinely separate events still each get their own.
 */
const claimedThisTick = new Set<NotifyCategory>();

function claimTick(category: NotifyCategory): boolean {
  if (claimedThisTick.has(category)) return false;
  claimedThisTick.add(category);
  queueMicrotask(() => claimedThisTick.delete(category));
  return true;
}

/** One context for the process: one per alert would leak a hardware audio stream each time. */
let context: AudioContext | undefined;

function audioContext(): AudioContext | undefined {
  if (context) return context;
  const Ctor = typeof AudioContext === "function" ? AudioContext : undefined;
  if (!Ctor) return undefined;
  try {
    context = new Ctor();
    return context;
  } catch {
    return undefined;
  }
}

/**
 * Decoded tones, keyed by URL.
 *
 * Decoding is the expensive part and a tone never changes while the app runs, so it is done once.
 * A failed fetch or decode is **not** kept, so a replacement that was briefly unreadable is tried
 * again on the next alert rather than written off for the process.
 */
const decodedTones = new Map<string, Promise<AudioBuffer | undefined>>();

async function decode(url: string, ctx: AudioContext): Promise<AudioBuffer | undefined> {
  let pending = decodedTones.get(url);
  if (!pending) {
    pending = (async () => {
      try {
        const response = await fetch(url);
        if (!response.ok) return undefined;
        return await ctx.decodeAudioData(await response.arrayBuffer());
      } catch {
        return undefined;
      }
    })();
    decodedTones.set(url, pending);
  }

  const buffer = await pending;
  if (!buffer) decodedTones.delete(url);
  return buffer;
}

/** Plays one source, resolving false if it couldn't be played at all. */
async function play(url: string, gain: number): Promise<boolean> {
  const ctx = audioContext();
  if (!ctx) return false;

  const buffer = await decode(url, ctx);
  if (!buffer) return false;

  try {
    // A context can start suspended; without this the first alert of a session is silent.
    if (ctx.state === "suspended") await ctx.resume();
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    const volume = ctx.createGain();
    volume.gain.value = gain;
    source.connect(volume);
    volume.connect(ctx.destination);
    source.start();
    return true;
  } catch {
    return false;
  }
}

/** Which file was actually heard — the answer the settings preview reports (#404). */
export type AlertSource = "replacement" | "bundled" | "none";

/**
 * Plays a category's tone: the user's replacement if there is a usable one, else the bundled default.
 *
 * A missing or corrupt override degrades to the standard sound rather than to silence, because
 * silence is indistinguishable from a broken feature. The one resolution path both an alert and the
 * settings preview go through, so "the preview plays what an alert would" is true by construction
 * rather than by comment.
 */
async function playResolved(category: NotifyCategory, gain: number): Promise<AlertSource> {
  const override = await overrideSoundUrl(category);
  if (override && (await play(override, gain))) return "replacement";
  if (await play(bundledSoundUrl(category), gain)) return "bundled";
  return "none";
}

/**
 * Sounds an alert for a category, if the platform allows it and the user asked for it.
 *
 * Never throws: failing to make a noise must not break the surface that triggered it.
 */
export async function playAlertSound(
  category: NotifyCategory,
  options: {enabled: boolean; volume?: string},
): Promise<boolean> {
  if (!options.enabled || !can("audioAlerts")) return false;
  // Claimed synchronously, before the first await, so an entire synchronous burst collapses to one.
  if (!claimTick(category)) return false;
  // `RootLayout` renders the notifiers in every whole-route window (#350), so without this one
  // ground stop played once per open window — the same tone, at once, near enough in phase.
  if (!(await isMainWindow())) return false;

  return (await playResolved(category, gainFor(options.volume))) !== "none";
}

/**
 * Plays a category on demand from the settings page, and says which file was heard (#404).
 *
 * Deliberately skips three of the guards {@link playAlertSound} needs, because each of them would be
 * wrong here:
 *
 * - the per-tick claim — pressing preview twice has to sound twice;
 * - the main-window check — the settings page may be open in a route window (#350);
 * - the category's own on/off switch — the point is to audition a tone *before* turning it on.
 *
 * The platform gate stays, so this is silent on the web build like everything else here.
 */
export async function previewAlertSound(
  category: NotifyCategory,
  volume?: string,
): Promise<AlertSource> {
  if (!can("audioAlerts")) return "none";
  return playResolved(category, gainFor(volume));
}

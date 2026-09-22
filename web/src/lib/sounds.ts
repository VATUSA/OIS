import {can} from "@/lib/platform";
import type {NotifyCategory} from "@/lib/desktop-notify";

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
 */

/** How loud, as the settings offer it. */
export type AlertVolume = "quiet" | "normal" | "loud";

const GAIN: Record<AlertVolume, number> = {
  quiet: 0.25,
  normal: 0.6,
  loud: 1,
};

export function gainFor(volume: string | undefined): number {
  return GAIN[(volume ?? "normal") as AlertVolume] ?? GAIN.normal;
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
 * trying to play it and falling back, rather than by asking the filesystem.
 */
async function resolveOverrideUrl(category: NotifyCategory): Promise<string | undefined> {
  try {
    const {appDataDir, join} = await import("@tauri-apps/api/path");
    const {convertFileSrc} = await import("@tauri-apps/api/core");
    return convertFileSrc(await join(await appDataDir(), "sounds", `${category}.wav`));
  } catch {
    return undefined;
  }
}

/**
 * Resolved override URLs, cached for the life of the process.
 *
 * The path does not move while the app is running, and almost nobody has dropped a file in — so
 * without this every single alert paid for two IPC round-trips before it could fall back to the
 * bundled tone. Only the *resolution* is cached; whether the file plays is still decided per alert,
 * so replacing the file takes effect without a restart.
 */
const overrideUrls = new Map<NotifyCategory, string | undefined>();

async function overrideSoundUrl(category: NotifyCategory): Promise<string | undefined> {
  if (!overrideUrls.has(category)) {
    overrideUrls.set(category, await resolveOverrideUrl(category));
  }
  return overrideUrls.get(category);
}

/**
 * Categories that have already claimed a sound in the current tick.
 *
 * `useNewKeys` calls `notifyDesktop` once per newly-appeared key, synchronously, so a batch — a TMU
 * releasing ten flights, or a role grant that `flattenPermissions` expands into dozens of
 * permission keys — used to start that many copies of one 0.38s tone at once. Summed amplitudes
 * clip and it reads as a blast rather than an alert. One sound per category per batch; the set is
 * emptied on the next microtask, so genuinely separate events still each get their own.
 */
const claimedThisTick = new Set<NotifyCategory>();

function claimTick(category: NotifyCategory): boolean {
  if (claimedThisTick.has(category)) return false;
  claimedThisTick.add(category);
  queueMicrotask(() => claimedThisTick.delete(category));
  return true;
}

/**
 * Whether this webview should be the one making the noise.
 *
 * `RootLayout` renders the notifiers in every whole-route window (#350), not just the main one, so
 * without this a ground stop played once per open window — the same tone, at once, near enough in
 * phase. `desktop-tray.tsx` and `popout.ts` guard app-wide resources the same way.
 */
async function isMainWindow(): Promise<boolean> {
  try {
    const {getCurrentWindow} = await import("@tauri-apps/api/window");
    return getCurrentWindow().label === "main";
  } catch {
    // Can't tell: stay quiet rather than risk one copy per window.
    return false;
  }
}

/** Plays one source, resolving false if it couldn't be played at all. */
function play(url: string, gain: number): Promise<boolean> {
  return new Promise((resolve) => {
    try {
      const audio = new Audio(url);
      audio.volume = gain;
      audio.onerror = () => resolve(false);
      // `play()` rejects if the source is missing or the platform refuses.
      void audio
        .play()
        .then(() => resolve(true))
        .catch(() => resolve(false));
    } catch {
      resolve(false);
    }
  });
}

/**
 * Sounds an alert for a category, if the platform allows it and the user asked for it.
 *
 * Tries the user's replacement first and falls back to the bundled default, so a missing or
 * corrupt override degrades to the standard sound rather than to silence — silence is
 * indistinguishable from a broken feature.
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
  if (!(await isMainWindow())) return false;

  const gain = gainFor(options.volume);

  const override = await overrideSoundUrl(category);
  if (override && (await play(override, gain))) return true;

  return play(bundledSoundUrl(category), gain);
}

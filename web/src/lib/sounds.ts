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
async function overrideSoundUrl(category: NotifyCategory): Promise<string | undefined> {
  try {
    const {appDataDir, join} = await import("@tauri-apps/api/path");
    const {convertFileSrc} = await import("@tauri-apps/api/core");
    return convertFileSrc(await join(await appDataDir(), "sounds", `${category}.wav`));
  } catch {
    return undefined;
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

  const gain = gainFor(options.volume);

  const override = await overrideSoundUrl(category);
  if (override && (await play(override, gain))) return true;

  return play(bundledSoundUrl(category), gain);
}

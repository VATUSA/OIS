import * as React from "react";

import {can} from "@/lib/platform";

/**
 * Signed auto-update for the desktop app (#347).
 *
 * The Tauri updater plugin does the part that matters for safety: it fetches the manifest, and
 * verifies each package's minisign signature against the public key compiled into the app. A
 * package that fails that check is **refused, never applied** — so there is no hand-rolled hash
 * check here, which would only be a weaker version of what the plugin already guarantees.
 *
 * What this module decides is *when*, and OIS is an operator console used during live events: an
 * update is fetched and verified in the background, then waits. The restart only ever happens on a
 * deliberate click.
 *
 * `@tauri-apps/plugin-*` is reached through dynamic `import()` for the same reason as the rest of
 * the platform seam — a static import would pull it into the web bundle (see `lib/platform.ts`).
 */

/** How often a long-running app re-checks. Most sessions never reach this; launch is the common path. */
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export type UpdateStatus =
  /** No update, or not the desktop app at all. */
  | {state: "idle"}
  /** Verified and staged; waiting for the user to accept the restart. */
  | {state: "ready"; version: string}
  /** The check or the download failed — including a signature that didn't verify. */
  | {state: "failed"};

/**
 * Checks for an update and, if there is one, downloads and verifies it.
 *
 * Resolves to `ready` only once the package is on disk **and** its signature has been verified by
 * the plugin, so a caller can treat `ready` as "safe to install".
 */
export async function fetchUpdate(): Promise<UpdateStatus> {
  if (!can("autoUpdate")) return {state: "idle"};

  try {
    const {check} = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return {state: "idle"};

    // Downloads and verifies; throws if the signature doesn't match the configured public key.
    await update.download();
    return {state: "ready", version: update.version};
  } catch {
    // A failed check is not worth interrupting anyone over — an unreachable feed, an offline
    // machine, or a package that failed verification all mean the same thing to the user: carry on
    // with the version they have.
    return {state: "failed"};
  }
}

/**
 * Applies the staged update and restarts.
 *
 * Only call this from a user action: it ends the current session, and doing that unasked to someone
 * running traffic is exactly what this design avoids.
 */
export async function installUpdate(): Promise<void> {
  const {check} = await import("@tauri-apps/plugin-updater");
  const {relaunch} = await import("@tauri-apps/plugin-process");

  const update = await check();
  if (!update) return;

  await update.downloadAndInstall();
  await relaunch();
}

/**
 * Update state for the UI: checks on mount, then periodically, and reports when one is staged.
 *
 * A no-op on the web build — `can("autoUpdate")` is false there, so nothing is imported or fetched.
 */
export function useDesktopUpdate(): UpdateStatus {
  const [status, setStatus] = React.useState<UpdateStatus>({state: "idle"});

  React.useEffect(() => {
    if (!can("autoUpdate")) return;

    let cancelled = false;
    const run = () => {
      void fetchUpdate().then((next) => {
        // Don't clobber a staged update with a later failed check — it is still installable.
        if (!cancelled && next.state !== "failed") setStatus(next);
      });
    };

    run();
    const timer = window.setInterval(run, CHECK_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  return status;
}

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

/** The staged package, held so installing applies the one we actually verified. */
type StagedUpdate = {install: () => Promise<void>};

export type UpdateStatus =
  /** No update, or not the desktop app at all. */
  | {state: "idle"}
  /** Verified and staged; waiting for the user to accept the restart. */
  | {state: "ready"; version: string; staged: StagedUpdate}
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
    // Hand back the very object we just verified. Re-`check()`ing at install time would download
    // the package a second time and apply whatever the feed serves *then* — not the thing the
    // banner told the user was verified.
    return {state: "ready", version: update.version, staged: update};
  } catch (error) {
    // An unreachable feed and a package that failed verification both leave the user on the
    // version they have, so the UI treats them alike — but they are not the same event, and a
    // signature that didn't match means someone served a package we refused. Say so somewhere.
    console.warn("[update] check or download failed", error);
    return {state: "failed"};
  }
}

/**
 * Applies the staged update and restarts.
 *
 * Only call this from a user action: it ends the current session, and doing that unasked to someone
 * running traffic is exactly what this design avoids.
 */
export async function installUpdate(staged: StagedUpdate): Promise<void> {
  const {relaunch} = await import("@tauri-apps/plugin-process");

  // `staged` is the package `fetchUpdate` already downloaded and verified, so this applies exactly
  // what the banner offered — no second download, and no window in which the feed could change
  // underneath the user between being told an update was ready and accepting it.
  await staged.install();
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

import * as React from "react";

import {can} from "@/lib/platform";
import {openRouteWindow} from "@/lib/popout";

/**
 * The OIS menu-bar presence (#351): status at a glance, quick links, and a way back to the app
 * when its window is hidden.
 *
 * The numbers come from the same hooks the dashboard renders (`useFeedStatus`, `useTmis`), passed
 * in by `components/desktop-tray.tsx`. That is deliberate: a tray with its own data source would
 * eventually disagree with the app, and a status readout that contradicts the screen beside it is
 * worse than no readout.
 *
 * Deliberately icon + tooltip only — no permanent text in the menu bar. A controller's menu bar is
 * already crowded, and a number that changes every 30 seconds competing for that space is the thing
 * people uninstall menu-bar apps over.
 */

export type TrayStatus = {
  /** Pilots on the network, or undefined while unknown. */
  pilots?: number;
  activeTmis?: number;
  /** Whether the VATSIM feed is healthy. `undefined` while unknown. */
  feedHealthy?: boolean;
};

/** The tray is a singleton; reusing the id means a re-sync updates it instead of stacking icons. */
const TRAY_ID = "ois-tray";

/**
 * The menu currently attached to the tray, so the one it replaces can be closed.
 *
 * `setMenu` does not dispose of the outgoing menu — it is a native resource held by the webview
 * that built it — and a fresh one is built on every status change.
 */
let current: {close?: () => Promise<void>} | undefined;

/** Where the quick links go. Chosen with you: the pages a controller actually lives in. */
const QUICK_LINKS: {label: string; route: string}[] = [
  {label: "TMU board", route: "/ops/tmu"},
  {label: "Facility map", route: "/facility-map"},
  {label: "FCAs · metering", route: "/ops/fca"},
  {label: "Advisories", route: "/advisories"},
];

function count(n: number | undefined): string {
  return n == null ? "—" : n.toLocaleString();
}

/** One-line summary for the icon's tooltip. */
export function trayTooltip(status: TrayStatus): string {
  const feed = status.feedHealthy == null ? "feed —" : status.feedHealthy ? "feed OK" : "feed stale";
  return `OIS · ${count(status.pilots)} pilots · ${count(status.activeTmis)} TMIs · ${feed}`;
}

/**
 * The status rows, as they appear at the top of the menu.
 *
 * Separated from the Tauri calls so what the user reads can be tested without a menu bar — and so
 * the wording lives in one place rather than being assembled inline.
 */
export function trayStatusRows(status: TrayStatus): string[] {
  return [
    `Pilots online: ${count(status.pilots)}`,
    `Active TMIs: ${count(status.activeTmis)}`,
    status.feedHealthy == null
      ? "Feed: unknown"
      : status.feedHealthy
        ? "Feed: healthy"
        : "Feed: stale",
  ];
}

/**
 * Tray work runs one call at a time. The status effect fires several times at startup as the feed
 * and TMI lists resolve, and two overlapping syncs each saw no icon and each created one: Tauri
 * does not keep tray ids unique, so two icons appeared, later syncs updated only the first, and
 * removing the tray left the second behind (VATUSA/OIS#351 review).
 */
let queue: Promise<unknown> = Promise.resolve();

function enqueue<T>(work: () => Promise<T>): Promise<T> {
  const run = queue.then(work);
  queue = run.catch(() => undefined);
  return run;
}

/**
 * Creates the tray if it isn't there, then updates its tooltip and menu to match `status`.
 *
 * A no-op on the web build. Never throws: a tray that fails to build must not take down the app
 * around it.
 */
export function syncTray(status: TrayStatus): Promise<boolean> {
  if (!can("tray")) return Promise.resolve(false);
  return enqueue(() => applyTray(status));
}

async function applyTray(status: TrayStatus): Promise<boolean> {
  try {
    const {TrayIcon} = await import("@tauri-apps/api/tray");
    const {Menu} = await import("@tauri-apps/api/menu");
    const {defaultWindowIcon} = await import("@tauri-apps/api/app");

    const existing = await TrayIcon.getById(TRAY_ID);
    // Checked before a menu is built, so a missing icon doesn't leak one menu per retry.
    const icon = existing ? undefined : ((await defaultWindowIcon()) ?? undefined);
    if (!existing && !icon) {
      // A tray icon with no image is invisible on macOS — the menu bar shows nothing at all, which
      // reads as "the feature is broken" rather than "the icon is missing".
      console.warn("OIS tray: no window icon available; the menu-bar icon would be invisible");
      return false;
    }

    const menu = await Menu.new({
      items: [
        // A readout, not actions — disabled so they don't look clickable.
        ...trayStatusRows(status).map((text) => ({text, enabled: false})),
        {item: "Separator"},
        {text: "Open OIS", action: () => void showMainWindow()},
        ...QUICK_LINKS.map((link) => ({
          text: link.label,
          action: () =>
            void openRouteWindow({id: link.route, title: `OIS · ${link.label}`, route: link.route}),
        })),
        {item: "Separator"},
        {text: "Quit OIS", action: () => void quit()},
      ],
    });

    if (existing) {
      await existing.setTooltip(trayTooltip(status));
      await existing.setMenu(menu);
    } else {
      await TrayIcon.new({
        id: TRAY_ID,
        icon,
        tooltip: trayTooltip(status),
        menu,
        // The menu is the whole point of the icon, so a left click should open it too.
        menuOnLeftClick: true,
      });
    }
    // Close the menu just replaced. Each sync builds a fresh one, and the pilot count moves every
    // ~30s, so without this a full event leaves hundreds of live menu handles behind.
    const previous = current;
    current = menu;
    await previous?.close?.().catch(() => undefined);
    return true;
  } catch (error) {
    // Surfaced rather than swallowed: the first version of this hid a missing Tauri permission,
    // and a silently-absent menu-bar icon gives no clue why.
    console.warn("OIS tray: could not build the menu-bar icon", error);
    return false;
  }
}

/**
 * Takes the icon out of the menu bar, for when the user turns the setting off.
 *
 * Queued behind any sync already in flight, so one that started before the tray was turned off
 * can't create the icon again after it has gone.
 */
export function removeTray(): Promise<void> {
  return enqueue(async () => {
    try {
      const {TrayIcon} = await import("@tauri-apps/api/tray");
      await TrayIcon.removeById(TRAY_ID);
      const previous = current;
      current = undefined;
      await previous?.close?.().catch(() => undefined);
    } catch {
      // Not there; nothing to remove.
    }
  });
}

/** Brings the main window back — the way back in once close-to-tray has hidden it. */
export async function showMainWindow(): Promise<void> {
  try {
    const {WebviewWindow} = await import("@tauri-apps/api/webviewWindow");
    const main = await WebviewWindow.getByLabel("main");
    if (!main) return;
    await main.show();
    await main.unminimize().catch(() => undefined);
    await main.setFocus();
  } catch {
    // Nothing to show.
  }
}

/**
 * Quits for real.
 *
 * Needed because with close-to-tray on, closing the window no longer ends the app — so the tray has
 * to offer the only remaining way out.
 */
export async function quit(): Promise<void> {
  try {
    const {exit} = await import("@tauri-apps/plugin-process");
    await exit(0);
  } catch {
    // Nothing sensible to do if even quitting fails.
  }
}

/**
 * This computer's login item, read from and written to the OS (#351) — see `LoginItemControl`.
 *
 * `enabled` is `undefined` until the OS has answered, so the settings row never shows a guess.
 * Setting it re-reads afterwards, so the switch reflects what the OS actually did rather than what
 * was asked for.
 */
export function useLoginItem(): {enabled: boolean | undefined; setEnabled: (on: boolean) => void} {
  const [enabled, setState] = React.useState<boolean | undefined>(undefined);

  const read = React.useCallback(async () => {
    try {
      const {isEnabled} = await import("@tauri-apps/plugin-autostart");
      setState(await isEnabled());
    } catch {
      setState(false); // No login items on this platform.
    }
  }, []);

  React.useEffect(() => {
    void read();
  }, [read]);

  const setEnabled = React.useCallback(
    (on: boolean) => {
      void (async () => {
        try {
          const {enable, disable} = await import("@tauri-apps/plugin-autostart");
          await (on ? enable() : disable());
        } catch {
          // Refused; the re-read below shows the switch where the OS left it.
        }
        await read();
      })();
    },
    [read],
  );

  return {enabled, setEnabled};
}

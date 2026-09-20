import * as React from "react";

import {useFeedStatus} from "@/lib/feed";
import {can} from "@/lib/platform";
import {useSetting} from "@/lib/settings";
import {removeTray, showMainWindow, syncTray} from "@/lib/tray";
import {useTmis} from "@/lib/tmu";

/**
 * Keeps the menu-bar tray in step with the app (#351), hides to it instead of quitting, and
 * reconciles the login item.
 *
 * Headless, mounted once in the root layout. The status comes from the *same* hooks the dashboard
 * tiles read, so the tray can't drift from what's on screen.
 *
 * Mounts nothing on the web build, where `can("tray")` is false.
 */
export function DesktopTray() {
  if (!can("tray")) return null;
  return <DesktopTrayInner />;
}

function DesktopTrayInner() {
  const {value: trayEnabled} = useSetting<boolean>("tray.show", false);
  const {value: closeToTray} = useSetting<boolean>("tray.closeToTray", false);
  const {value: launchAtLogin} = useSetting<boolean>("tray.launchAtLogin", false);

  return (
    <>
      {/* Only mounted when the tray is on: it opens polling queries, and someone who never turns
          the tray on shouldn't pay for them. Unmounting takes the icon away. */}
      {trayEnabled ? <TrayStatus /> : <TrayAbsent />}
      <CloseToTray enabled={closeToTray} />
      <LaunchAtLogin enabled={launchAtLogin} />
    </>
  );
}

/** Pushes the live numbers into the tray whenever they move. */
function TrayStatus() {
  const feed = useFeedStatus();
  const tmis = useTmis();

  React.useEffect(() => {
    void syncTray({
      pilots: feed.data?.pilots,
      activeTmis: tmis.data?.length,
      feedHealthy: feed.data?.healthy,
    });
  }, [feed.data?.pilots, feed.data?.healthy, tmis.data?.length]);

  return null;
}

/** Takes the icon out of the menu bar once the setting is turned off. */
function TrayAbsent() {
  React.useEffect(() => {
    void removeTray();
  }, []);
  return null;
}

/**
 * Closing the window hides it rather than ending the app, so notifications keep arriving.
 *
 * Main window only: #350's route windows use `onCloseRequested` to forget themselves, and
 * intercepting those would leave windows the user cannot close.
 */
function CloseToTray({enabled}: {enabled: boolean}) {
  React.useEffect(() => {
    // Turning this off while the window is hidden would strand the user with no way back.
    if (!enabled) {
      void showMainWindow();
      return;
    }

    let dispose: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      try {
        const {getCurrentWindow} = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        if (win.label !== "main") return;

        const unlisten = await win.onCloseRequested((event) => {
          event.preventDefault();
          void win.hide();
        });
        if (cancelled) unlisten();
        else dispose = unlisten;
      } catch {
        // Without the listener the window closes normally, which is the previous behaviour.
      }
    })();

    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [enabled]);

  return null;
}

/**
 * Reconciles the OS login item with the setting.
 *
 * Reconciled rather than set blindly: the user may have removed the login item themselves, and
 * writing it back on every launch would be us overruling that.
 */
function LaunchAtLogin({enabled}: {enabled: boolean}) {
  React.useEffect(() => {
    void (async () => {
      try {
        const {enable, disable, isEnabled} = await import("@tauri-apps/plugin-autostart");
        const already = await isEnabled();
        if (enabled && !already) await enable();
        if (!enabled && already) await disable();
      } catch {
        // No autostart on this platform, or it refused; the setting simply doesn't take effect.
      }
    })();
  }, [enabled]);

  return null;
}

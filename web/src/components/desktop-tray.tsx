import * as React from "react";

import {safeUnlisten} from "@/lib/desktop-events";
import {useFeedStatus} from "@/lib/feed";
import {can, isMainWindow} from "@/lib/platform";
import {useSetting} from "@/lib/settings";
import {removeTray, showMainWindow, syncTray} from "@/lib/tray";
import {isActiveTmi, useTmis} from "@/lib/tmu";

/**
 * Keeps the menu-bar tray in step with the app (#351), and hides to it instead of quitting.
 *
 * Launching at login is not here: it is a property of this computer, read and written straight
 * from the OS by its settings row (`useLoginItem`), never reconciled at launch.
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

  return (
    <>
      {/* Only mounted when the tray is on: it opens polling queries, and someone who never turns
          the tray on shouldn't pay for them. Unmounting takes the icon away. */}
      {trayEnabled ? <TrayStatus /> : <TrayAbsent />}
      {/* Hiding to the tray needs the tray: with the icon off, a hidden window had no way back and
          no Quit — on Windows and Linux there is no dock to click (VATUSA/OIS#351 review). */}
      <CloseToTray enabled={closeToTray && trayEnabled} />
    </>
  );
}

/** Pushes the live numbers into the tray whenever they move. */
function TrayStatus() {
  // Polls while hidden: the window being hidden to the tray is when these numbers are read.
  const feed = useFeedStatus({background: true});
  const tmis = useTmis();
  // Live TMIs only — the dashboard's count, not every TMI the list endpoint returns.
  const activeTmis = tmis.data?.filter(isActiveTmi).length;

  React.useEffect(() => {
    void syncTray({
      pilots: feed.data?.pilots,
      activeTmis,
      feedHealthy: feed.data?.healthy,
    });
  }, [feed.data?.pilots, feed.data?.healthy, activeTmis]);

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
export function CloseToTray({enabled}: {enabled: boolean}) {
  React.useEffect(() => {
    let dispose: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      try {
        const {getCurrentWindow} = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        // Both branches below are about the MAIN window, and the check has to come first: an
        // unguarded `showMainWindow()` meant every route window (#350) dragged focus back to the
        // main one as it opened — on the default settings, since closeToTray is off by default.
        // `PrimaryWindowFeatures` also keeps this out of route windows; this is the belt to that
        // pair of braces, and the rule itself lives in `platform.ts` (#403).
        if (!(await isMainWindow())) return;

        // Turning this off while the window is hidden would strand the user with no way back.
        if (!enabled) {
          await showMainWindow();
          return;
        }

        const unlisten = await win.onCloseRequested((event) => {
          event.preventDefault();
          void win.hide();
        });
        if (cancelled) safeUnlisten(unlisten);
        else dispose = unlisten;
      } catch {
        // Without the listener the window closes normally, which is the previous behaviour.
      }
    })();

    return () => {
      cancelled = true;
      safeUnlisten(dispose);
    };
  }, [enabled]);

  return null;
}

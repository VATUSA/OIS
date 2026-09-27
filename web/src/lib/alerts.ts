import {isTauri} from "@/lib/platform";

/**
 * A way to ask the on-screen restriction alerts to clear (#352).
 *
 * `restriction-alerts.tsx` owns its alert list in local state, which is right — nothing else should
 * be able to *add* to it. But a global hotkey needs to dismiss what's showing without the user
 * hunting for each popup, and that hotkey fires from outside React entirely.
 *
 * So: a one-line subscription rather than lifting that state into a context. The component keeps
 * ownership; this only lets something ask it to empty.
 */

type Listener = () => void;

const listeners = new Set<Listener>();

/**
 * The same request, for every desktop window. The hotkey fires in the main window's JS, but alerts
 * show in every window (#350) — calling only this window's listeners left an alert up in the route
 * window the controller was actually looking at (VATUSA/OIS#352 review).
 */
const DISMISS_EVENT = "ois://alerts/dismiss-all";

/** Subscribes to dismiss-all requests, from this window or any other. Returns an unsubscribe. */
export function onDismissAllAlerts(listener: Listener): () => void {
  listeners.add(listener);
  let offDesktop: (() => void) | undefined;
  let removed = false;
  if (isTauri()) {
    void import("@tauri-apps/api/event")
      .then(({listen}) => listen(DISMISS_EVENT, () => listener()))
      .then((off) => {
        if (removed) off();
        else offDesktop = off;
      })
      .catch(() => undefined);
  }
  return () => {
    removed = true;
    listeners.delete(listener);
    offDesktop?.();
  };
}

/** Asks every mounted alert surface, in every window, to clear what it is showing. */
export function dismissAllAlerts() {
  for (const listener of listeners) listener();
  if (isTauri()) {
    void import("@tauri-apps/api/event")
      .then(({emit}) => emit(DISMISS_EVENT))
      .catch(() => undefined);
  }
}

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

/** Subscribes to dismiss-all requests. Returns an unsubscribe. */
export function onDismissAllAlerts(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Asks every mounted alert surface to clear what it is showing. */
export function dismissAllAlerts() {
  for (const listener of listeners) listener();
}

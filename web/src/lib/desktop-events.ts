/**
 * Running a Tauri unlisten handle without letting its failure escape (#426).
 *
 * `listen()` resolves to an **async** function — `async () => _unlisten(event, eventId)` in
 * `@tauri-apps/api/event.js` — whose first act is
 * `__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, eventId)`. That reads
 * `listeners[eventId].handlerId` and throws when the webview no longer knows the id.
 *
 * Because the throw happens *inside an async function* it never arrives as an exception — it becomes
 * a rejected promise. A caller that writes `unlisten?.()` and moves on has discarded that rejection,
 * so it surfaces as `[Unhandled rejection] TypeError: undefined is not an object`. This is also why
 * wrapping the call in `try`/`catch` does nothing: there is no synchronous throw to catch.
 *
 * Tearing a listener down is best-effort — it happens as a component unmounts or a window goes away,
 * and a listener outliving its handler is harmless next to a crash report. So the failure is
 * swallowed on purpose. What it must not do is stay *unhandled*.
 */
export function safeUnlisten(unlisten: (() => unknown) | undefined): void {
  try {
    void Promise.resolve(unlisten?.()).catch(() => {
      // Already unregistered, or the webview forgot the id. Either way it is not coming back.
    });
  } catch {
    // A handle that throws synchronously matters no more than one that rejects.
  }
}

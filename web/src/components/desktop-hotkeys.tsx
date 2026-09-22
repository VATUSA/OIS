import * as React from "react";
import {useToast} from "@ois/ui";

import {
  applyHotkeys,
  clearHotkeys,
  HOTKEY_ACTIONS,
  onHotkeysResume,
  type HotkeyAction,
} from "@/lib/hotkeys";
import {can, isMainWindow} from "@/lib/platform";
import {useSettings} from "@/lib/settings";

/**
 * Registers the user's global shortcuts and keeps them in step with the settings (#352).
 *
 * Headless, mounted in the root layout — which the desktop shell renders in *every* whole-route
 * window (#350), not just the main one. A shortcut is registered with the OS, not with a window,
 * and `unregisterAll()` is app-scoped, so a second window doing this would take the main window's
 * shortcuts over and then release them all when it closed. Hence the `isMainWindow` gate below;
 * `popout.ts`'s `restoreWindows` and `desktop-tray.tsx` guard the same way for the same reason.
 *
 * Nothing runs on the web build: `can("globalHotkeys")` is false there.
 */
export function DesktopHotkeys() {
  if (!can("globalHotkeys")) return null;
  return <DesktopHotkeysInner />;
}

/** How long typing must pause before a binding is handed to the OS. */
const SETTLE_MS = 900;

function DesktopHotkeysInner() {
  const settings = useSettings();
  const toast = useToast();
  // `null` until we know. Never act on "not yet known" — registering first and discovering we are
  // a route window afterwards is the bug this gate exists to prevent.
  const [isMain, setIsMain] = React.useState<boolean | null>(null);
  React.useEffect(() => {
    let alive = true;
    void isMainWindow().then((main) => {
      if (alive) setIsMain(main);
    });
    return () => {
      alive = false;
    };
  }, []);

  // Bumped when a settings field stops capturing a keystroke, so the bindings it released are
  // taken back even if the user cancelled without changing anything.
  const [resumeNonce, setResumeNonce] = React.useState(0);
  React.useEffect(() => onHotkeysResume(() => setResumeNonce((n) => n + 1)), []);

  // The accelerators as a stable string, so the effect re-runs when a binding actually changes
  // rather than on every settings refetch.
  const bindings = React.useMemo(() => {
    const out: Partial<Record<HotkeyAction, string>> = {};
    for (const {action, settingKey} of HOTKEY_ACTIONS) {
      const value = settings.data?.[settingKey];
      if (typeof value === "string" && value.trim()) out[action] = value.trim();
    }
    return out;
  }, [settings.data]);

  const signature = JSON.stringify(bindings);
  // Only complain about a given combination once; re-registering happens on every settings change.
  const warned = React.useRef<string>("");

  React.useEffect(() => {
    // Only the main window owns the OS-level registrations — see the note on the component.
    if (isMain !== true) return;
    // Don't clear the user's shortcuts while their settings are still loading — that would
    // unregister everything on every launch for as long as the request takes.
    if (!settings.isSuccess) return;

    let cancelled = false;

    // Wait for typing to stop before touching the OS.
    //
    // These settings are typed character by character, and each keystroke changes the binding.
    // Registering immediately means the half-finished value briefly becomes a live *global*
    // shortcut — so typing "CommandOrControl+Shift+O" registers "C" for a moment, and that
    // registration then swallows the next keystroke from the very field being typed into. The
    // accelerator can never be finished.
    const timer = window.setTimeout(() => {
      void applyHotkeys(bindings).then((results) => {
        if (cancelled) return;

        const refused = results.filter((r) => !r.registered);
        if (refused.length && warned.current !== signature) {
          warned.current = signature;
          // A shortcut that silently does nothing is the worst outcome: the user presses it,
          // nothing happens, and there is nothing anywhere to read.
          // Both reasons, not whichever came first: a no-modifier binding used to hide an
          // "already taken" one entirely, and the dedupe below then meant that conflict was never
          // reported again for this configuration.
          const reasons: string[] = [];
          const noModifier = refused.filter((r) => r.reason === "no-modifier");
          const unavailable = refused.filter((r) => r.reason !== "no-modifier");
          if (noModifier.length) {
            reasons.push(
              `${noModifier.map((r) => r.accelerator).join(", ")} — a shortcut needs a modifier such as Command or Control, or it would fire whenever you type that key anywhere.`,
            );
          }
          if (unavailable.length) {
            reasons.push(
              `${unavailable.map((r) => r.accelerator).join(", ")} — another application may already use them.`,
            );
          }
          toast.warning("Some shortcuts couldn't be registered", {
            description: reasons.join(" "),
          });
        }
      });
    }, SETTLE_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
    // `toast` is stable; re-running on it would re-register on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature, settings.isSuccess, isMain, resumeNonce]);

  // Release the combinations back to the OS when the app tears this down — but only from the window
  // that took them, or closing a route window would unregister the main window's shortcuts too.
  React.useEffect(() => {
    if (isMain !== true) return;
    return () => void clearHotkeys();
  }, [isMain]);

  return null;
}

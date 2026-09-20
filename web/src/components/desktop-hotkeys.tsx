import * as React from "react";
import {useToast} from "@ois/ui";

import {applyHotkeys, clearHotkeys, HOTKEY_ACTIONS, type HotkeyAction} from "@/lib/hotkeys";
import {can} from "@/lib/platform";
import {useSettings} from "@/lib/settings";

/**
 * Registers the user's global shortcuts and keeps them in step with the settings (#352).
 *
 * Headless, mounted once in the root layout, and only in the main window — a shortcut is registered
 * with the OS, not with a window, so every window doing it would fight over the same combinations.
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
          const noModifier = refused.filter((r) => r.reason === "no-modifier");
          toast.warning("Some shortcuts couldn't be registered", {
            description: noModifier.length
              ? `${noModifier.map((r) => r.accelerator).join(", ")} — a shortcut needs a modifier such as Command or Control, or it would fire whenever you type that key anywhere.`
              : `${refused.map((r) => r.accelerator).join(", ")} — another application may already use them.`,
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
  }, [signature, settings.isSuccess]);

  // Release the combinations back to the OS when the app tears this down.
  React.useEffect(() => () => void clearHotkeys(), []);

  return null;
}

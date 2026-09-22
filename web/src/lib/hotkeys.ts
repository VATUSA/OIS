import {dismissAllAlerts} from "@/lib/alerts";
import {can} from "@/lib/platform";
import {openRouteWindow} from "@/lib/popout";
import {showMainWindow} from "@/lib/tray";

/**
 * OS-level global shortcuts (#352) — they fire while another application is focused, which is the
 * whole point: a controller is usually in CRC or vATIS, not in OIS, when something happens.
 *
 * Accelerators are typed by the user rather than picked from a list. Controllers run other software
 * that already claims the obvious combinations, so a fixed set of presets could plausibly be
 * entirely unusable on a real desk.
 *
 * Every action here reuses something that already exists: focusing is #351's `showMainWindow`,
 * jumping is #350's `openRouteWindow`, and dismissing is the alert seam in `lib/alerts.ts`.
 */

export type HotkeyAction =
  | "focus"
  | "dismissAlerts"
  | "tmu"
  | "facilityMap"
  | "fca"
  | "advisories";

/** Setting key and human label for each bindable action, in the order they appear in settings. */
export const HOTKEY_ACTIONS: {action: HotkeyAction; settingKey: string; label: string}[] = [
  {action: "focus", settingKey: "hotkeys.focus", label: "Bring OIS to the front"},
  {action: "dismissAlerts", settingKey: "hotkeys.dismissAlerts", label: "Dismiss alerts"},
  {action: "tmu", settingKey: "hotkeys.tmu", label: "Open TMU board"},
  {action: "facilityMap", settingKey: "hotkeys.facilityMap", label: "Open facility map"},
  {action: "fca", settingKey: "hotkeys.fca", label: "Open FCAs · metering"},
  {action: "advisories", settingKey: "hotkeys.advisories", label: "Open advisories"},
];

const ROUTES: Partial<Record<HotkeyAction, {route: string; title: string}>> = {
  tmu: {route: "/ops/tmu", title: "OIS · TMU board"},
  facilityMap: {route: "/facility-map", title: "OIS · Facility map"},
  fca: {route: "/ops/fca", title: "OIS · FCAs"},
  advisories: {route: "/advisories", title: "OIS · Advisories"},
};

/** What each shortcut does when pressed. */
export function runHotkey(action: HotkeyAction) {
  if (action === "focus") {
    void showMainWindow();
    return;
  }
  if (action === "dismissAlerts") {
    dismissAllAlerts();
    return;
  }

  const target = ROUTES[action];
  if (target) void openRouteWindow({id: target.route, ...target});
}

/**
 * Tidies a typed accelerator.
 *
 * Deliberately no parsing beyond whitespace: what makes an accelerator usable isn't whether it
 * *looks* right, it's whether the OS will hand it over — another application may already own it.
 * That question is answered by {@link applyHotkeys} actually attempting the registration.
 */
export function normalizeAccelerator(raw: string | undefined): string {
  return (raw ?? "").trim();
}

/** Modifier names the plugin understands, lowercased for comparison. */
const MODIFIERS = new Set([
  "command",
  "cmd",
  "control",
  "ctrl",
  "commandorcontrol",
  "cmdorctrl",
  "alt",
  "option",
  "altgr",
  "shift",
  "super",
  "meta",
]);

/**
 * Whether an accelerator carries at least one modifier.
 *
 * A modifier-less shortcut is *global*: bind `O` and OIS reacts every time you type the letter O in
 * any application, including the field you typed it into. That is never what someone means, so it
 * is refused with a reason rather than registered and left to cause confusion.
 */
export function hasModifier(accelerator: string): boolean {
  const parts = accelerator.split("+").map((p) => p.trim().toLowerCase()).filter(Boolean);
  return parts.length > 1 && parts.slice(0, -1).some((p) => MODIFIERS.has(p));
}

/** Per-shortcut outcome, so settings can show which ones the OS actually gave us. */
export type HotkeyResult = {
  action: HotkeyAction;
  accelerator: string;
  registered: boolean;
  /** Why it wasn't taken, when it wasn't. */
  reason?: "no-modifier" | "unavailable";
};

/**
 * Replaces every registered shortcut with the given bindings.
 *
 * Unregisters everything first: editing one binding must not leave the previous key combination
 * live, which is the bug people notice weeks later when an old shortcut still fires.
 *
 * A no-op on the web build. Never throws — a shortcut that can't be taken is reported, not fatal.
 */
export async function applyHotkeys(
  bindings: Partial<Record<HotkeyAction, string>>,
): Promise<HotkeyResult[]> {
  if (!can("globalHotkeys")) return [];

  try {
    const {register, unregisterAll} = await import("@tauri-apps/plugin-global-shortcut");
    await unregisterAll();

    const results: HotkeyResult[] = [];
    for (const {action} of HOTKEY_ACTIONS) {
      const accelerator = normalizeAccelerator(bindings[action]);
      // An unset shortcut is not a failure — it is the default, and nobody should have a global
      // key combination taken from them by installing an update.
      if (!accelerator) continue;

      if (!hasModifier(accelerator)) {
        results.push({action, accelerator, registered: false, reason: "no-modifier"});
        continue;
      }

      try {
        await register(accelerator, (event) => {
          // The plugin fires for press *and* release; acting on both runs everything twice.
          if (event.state === "Pressed") runHotkey(action);
        });
        results.push({action, accelerator, registered: true});
      } catch {
        // Almost always "another application already owns this combination" — the failure a
        // controller will actually hit, and one they can only fix if they're told about it.
        results.push({action, accelerator, registered: false, reason: "unavailable"});
      }
    }
    return results;
  } catch {
    return [];
  }
}

/**
 * Lets the settings UI hand the key combinations back while it records a new one.
 *
 * A registered global shortcut is swallowed by the OS before the webview sees it, so pressing the
 * combination that is *already* bound — the obvious thing to do when moving it to another action —
 * would fire the old shortcut and record nothing. The field therefore suspends the shortcuts while
 * it captures, and resumes when it stops; the resume is a signal rather than a re-registration
 * because only `DesktopHotkeys` knows the current bindings.
 */
const resumeListeners = new Set<() => void>();

/** Subscribes to "capture finished, take the shortcuts back". Returns an unsubscribe. */
export function onHotkeysResume(listener: () => void): () => void {
  resumeListeners.add(listener);
  return () => {
    resumeListeners.delete(listener);
  };
}

/** Hands every combination back to the OS while a new one is being recorded. */
export async function suspendHotkeys(): Promise<void> {
  await clearHotkeys();
}

/** Asks whoever owns the bindings to register them again. */
export function resumeHotkeys(): void {
  for (const listener of resumeListeners) listener();
}

/** Releases every shortcut — used when the feature is switched off. */
export async function clearHotkeys(): Promise<void> {
  if (!can("globalHotkeys")) return;
  try {
    const {unregisterAll} = await import("@tauri-apps/plugin-global-shortcut");
    await unregisterAll();
  } catch {
    // Nothing registered.
  }
}

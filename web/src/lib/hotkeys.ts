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
 * any application, including the field you typed it into. `Shift` alone is the same thing with a
 * capital letter, so it doesn't count. That is never what someone means, so it
 * is refused with a reason rather than registered and left to cause confusion.
 */
export function hasModifier(accelerator: string): boolean {
  const parts = accelerator.split("+").map((p) => p.trim().toLowerCase()).filter(Boolean);
  // Shift alone doesn't count: Shift+O is just a capital O, so it would take that letter away from
  // every application on the machine (VATUSA/OIS#352 review).
  return parts.length > 1 && parts.slice(0, -1).some((p) => MODIFIERS.has(p) && p !== "shift");
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
 * Registration runs one call at a time. An apply is a sequence of `register` calls; overlapping it
 * with a suspend's `unregisterAll` (or another apply) let the rest of the sequence register after
 * everything had been released (VATUSA/OIS#352 review).
 */
let queue: Promise<unknown> = Promise.resolve();

function enqueue<T>(work: () => Promise<T>): Promise<T> {
  const run = queue.then(work);
  queue = run.catch(() => undefined);
  return run;
}

/**
 * True while a settings field anywhere is recording a new shortcut. Only meaningful in the window
 * that owns the registrations — see {@link ownHotkeys}.
 */
let suspended = false;

/**
 * Replaces every registered shortcut with the given bindings.
 *
 * Unregisters everything first: editing one binding must not leave the previous key combination
 * live, which is the bug people notice weeks later when an old shortcut still fires.
 *
 * Registers nothing while shortcuts are suspended — checked before every `register`, so a suspend
 * that arrives part-way through an apply stops the rest of it too.
 *
 * A no-op on the web build. Never throws — a shortcut that can't be taken is reported, not fatal.
 */
export function applyHotkeys(
  bindings: Partial<Record<HotkeyAction, string>>,
): Promise<HotkeyResult[]> {
  if (!can("globalHotkeys")) return Promise.resolve([]);
  return enqueue(() => registerAll(bindings));
}

async function registerAll(
  bindings: Partial<Record<HotkeyAction, string>>,
): Promise<HotkeyResult[]> {
  try {
    const {register, unregisterAll} = await import("@tauri-apps/plugin-global-shortcut");
    await unregisterAll();

    const results: HotkeyResult[] = [];
    for (const {action} of HOTKEY_ACTIONS) {
      if (suspended) return results; // a field started recording: leave the combinations free
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

/** Releases every shortcut — used when the feature is switched off, and while one is recorded. */
export function clearHotkeys(): Promise<void> {
  if (!can("globalHotkeys")) return Promise.resolve();
  return enqueue(async () => {
    try {
      const {unregisterAll} = await import("@tauri-apps/plugin-global-shortcut");
      await unregisterAll();
    } catch {
      // Nothing registered.
    }
  });
}

/**
 * Suspending and resuming are app-wide events rather than calls, because the settings field that
 * asks may be in a different window from the one that owns the shortcuts. As in-window calls, a
 * field in a route window (#350) released every shortcut and its "take them back" never reached the
 * main window, so all of them stayed dead (VATUSA/OIS#352 review).
 */
const SUSPEND_EVENT = "ois://hotkeys/suspend";
const RESUME_EVENT = "ois://hotkeys/resume";

async function broadcast(event: string): Promise<void> {
  if (!can("globalHotkeys")) return;
  try {
    const {emit} = await import("@tauri-apps/api/event");
    await emit(event);
  } catch {
    // No shell to tell; nothing is registered either.
  }
}

/**
 * Hands every combination back to the OS while a new one is being recorded.
 *
 * A registered global shortcut is swallowed by the OS before the webview sees it, so pressing the
 * combination that is *already* bound — the obvious thing to do when moving it to another action —
 * would fire the old shortcut and record nothing.
 */
export function suspendHotkeys(): Promise<void> {
  return broadcast(SUSPEND_EVENT);
}

/** Recording finished: asks the owner to register the (possibly changed) bindings again. */
export function resumeHotkeys(): Promise<void> {
  return broadcast(RESUME_EVENT);
}

/**
 * Makes this window the owner of the shortcuts' suspend/resume: suspending releases everything and
 * holds it released; resuming lifts that and calls `onResume`, which should re-read the bindings —
 * a field in another window may just have changed one. Returns a disposer.
 */
export async function ownHotkeys(onResume: () => void): Promise<() => void> {
  if (!can("globalHotkeys")) return () => undefined;
  try {
    const {listen} = await import("@tauri-apps/api/event");
    const offSuspend = await listen(SUSPEND_EVENT, () => {
      suspended = true;
      void clearHotkeys();
    });
    const offResume = await listen(RESUME_EVENT, () => {
      suspended = false;
      onResume();
    });
    return () => {
      offSuspend();
      offResume();
    };
  } catch {
    return () => undefined;
  }
}

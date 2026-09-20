import {can} from "@/lib/platform";
import {playAlertSound} from "@/lib/sounds";

/**
 * Native OS notifications for the desktop app (#348).
 *
 * The single place a notification is raised, so every category goes through the same gates:
 * the platform must support it, the user must have opted that category in, and the OS must have
 * granted permission. On the web build `can("notifications")` is false and nothing here runs —
 * no import, no permission prompt, no notification.
 *
 * What is worth notifying about is decided by the detectors that call this, in the browser, where
 * the user's own permissions and facility already apply. That matters: the websocket those
 * detectors react to is broadcast unfiltered to every signed-in client, so targeting *has* to
 * happen here rather than on the wire.
 *
 * `@tauri-apps/plugin-notification` is reached through a dynamic `import()` so it stays out of the
 * web bundle — the rule `eslint.config.mjs` enforces for every `@tauri-apps/*` package.
 */

/** Notification categories, each independently opt-in. Keys match the settings registry. */
export type NotifyCategory =
  | "restrictions"
  | "releases"
  | "metering"
  | "access"
  | "eventReminders";

export type Notification = {
  category: NotifyCategory;
  title: string;
  body: string;
  /** In-app route to open when the notification is clicked. */
  route: string;
};

/** Where a click should take the user — read back from the notification's `extra` payload. */
const ROUTE_KEY = "ois.route";

/**
 * Asks the OS once, lazily.
 *
 * Deliberately on first *use* rather than at launch: a permission prompt the moment the app opens,
 * before the user has asked for anything, is the kind of thing people deny out of hand.
 */
let permission: Promise<boolean> | undefined;

async function ensurePermission(): Promise<boolean> {
  permission ??= (async () => {
    try {
      const {isPermissionGranted, requestPermission} = await import(
        "@tauri-apps/plugin-notification"
      );
      if (await isPermissionGranted()) return true;
      return (await requestPermission()) === "granted";
    } catch {
      return false;
    }
  })();

  return permission;
}

/**
 * Raises a native notification, if the platform, the user's settings and the OS all allow it.
 *
 * `enabled` is passed in rather than read here because settings live behind a React hook; the
 * caller is already in a component and has it.
 *
 * Never throws: a notification failing is not a reason to break the surface that triggered it.
 */
export async function notifyDesktop(
  notification: Notification,
  enabled: boolean,
  sound?: {enabled: boolean; volume?: string},
): Promise<boolean> {
  // Sound is a second output of the same detection, with its own settings — a category can notify
  // silently, or make a noise without a banner (#353). Fired before the permission check below,
  // which is about *notifications* and has nothing to say about audio.
  if (sound?.enabled) void playAlertSound(notification.category, sound);

  if (!enabled || !can("notifications")) return false;
  if (!(await ensurePermission())) return false;

  try {
    const {sendNotification} = await import("@tauri-apps/plugin-notification");
    sendNotification({
      title: notification.title,
      body: notification.body,
      extra: {[ROUTE_KEY]: notification.route},
    });
    return true;
  } catch {
    return false;
  }
}

/**
 * Starts listening for notification clicks: raises the window and hands the route to `navigate`.
 *
 * Unminimise *and* show *and* focus, in that order — a backgrounded app needs a different one of
 * those on each platform, and doing all three is both harmless and the only reliable way to end up
 * actually in front of the user.
 *
 * Returns a disposer, or undefined on the web build where there is nothing to listen to.
 */
export async function listenForNotificationClicks(
  navigate: (route: string) => void,
): Promise<(() => void) | undefined> {
  if (!can("notifications")) return undefined;

  try {
    const {onAction} = await import("@tauri-apps/plugin-notification");
    const listener = await onAction(async (notification) => {
      const route = notification.extra?.[ROUTE_KEY];

      try {
        const {getCurrentWindow} = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        await win.unminimize();
        await win.show();
        await win.setFocus();
      } catch {
        // Raising the window failed; still navigate, so the app is at least on the right page
        // when the user gets to it.
      }

      if (typeof route === "string" && route) navigate(route);
    });

    return () => listener.unregister();
  } catch {
    return undefined;
  }
}

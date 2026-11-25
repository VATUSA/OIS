/**
 * Labels for keyboard-shortcut hints: "⌘K" on Apple platforms, "Ctrl K" everywhere else. The hotkey
 * handlers accept either modifier; this decides which one the hint names (#1005).
 */

/** `navigator.userAgentData` is not in TypeScript's DOM lib; only `platform` is read. */
interface NavigatorWithUAData {
  userAgentData?: {platform?: string};
  platform?: string;
}

/**
 * The OS name, read once at module load. Chromium, the Windows desktop webview included, answers
 * through `userAgentData` in a secure context ("macOS", "Windows"); elsewhere, and in WebKit (the
 * macOS and Linux desktop webviews), `navigator.platform` answers ("MacIntel", "Win32",
 * "Linux x86_64").
 */
function readPlatform(): string {
  if (typeof navigator === "undefined") return "";
  const nav = navigator as NavigatorWithUAData;
  return nav.userAgentData?.platform ?? nav.platform ?? "";
}

const PLATFORM = readPlatform();

/** Whether a platform string names an Apple OS, where the Command key is the shortcut modifier. */
export function isApplePlatform(platform: string): boolean {
  return /mac|iphone|ipad/i.test(platform);
}

/** The hint for a Command/Control shortcut on `key`, e.g. `shortcutLabel("K")`. */
export function shortcutLabel(key: string, platform: string = PLATFORM): string {
  return isApplePlatform(platform) ? `⌘${key}` : `Ctrl ${key}`;
}

import type {Capability} from "./platform";

/**
 * Whether each capability is actually *implemented* yet, independent of which platform we're on.
 *
 * Every entry starts `false` and is flipped by the issue that builds it, so the platform seam can
 * never claim an ability the app doesn't have. Gating on a capability is therefore safe to write
 * today and starts working the day its feature lands — no caller changes needed.
 *
 * Its own module so a test can substitute it (VATUSA/OIS#345): while every flag is `false`,
 * `can()` answers `false` whether or not it consults the desktop gate, so only a test that turns a
 * flag on can see a `can()` that has stopped checking — which is how desktop-only UI would reach
 * the browser the day #348 flips the first one.
 */
export const IMPLEMENTED: Readonly<Record<Capability, boolean>> = Object.freeze({
  autoUpdate: true, // #347 — signed auto-update (shipped)
  notifications: true, // #348 — native OS notifications (shipped)
  miniWindows: true, // #349 — pop-out always-on-top mini-windows (shipped)
  multiWindow: false, // #350 — multi-window / multi-monitor
  tray: false, // #351 — system tray
  globalHotkeys: false, // #352 — global hotkeys
  audioAlerts: false, // #353 — audio alerts
  fileDialogs: false, // #354 — native export/import dialogs
});

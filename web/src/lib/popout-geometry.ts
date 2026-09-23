/**
 * Pure geometry maths for pop-out windows (#349), kept apart from the Tauri calls so it can be
 * tested without a window.
 */

/**
 * A window rect in **logical** pixels — the unit `WebviewWindow` options take, and the unit saved
 * geometry is stored in.
 *
 * Tauri *reports* window position and size in physical pixels (`outerPosition`, `outerSize`), so they
 * go through {@link toLogical} before being saved. The two units coincide on a 1x display and differ
 * by the scale factor everywhere else: storing physical and restoring it as logical doubled a pop-out
 * on every reopen on a 2x Retina screen, compounding each cycle (VATUSA/OIS#349 review).
 */
export type Rect = {x: number; y: number; width: number; height: number};

/** A monitor as Tauri reports it: position and size in **physical** pixels, and its scale factor. */
export type Monitor = {
  position: {x: number; y: number};
  size: {width: number; height: number};
  scaleFactor: number;
};

/** A physical-pixel rect as a logical one, at `scaleFactor`. */
export function toLogical(physical: Rect, scaleFactor: number): Rect {
  const f = scaleFactor > 0 ? scaleFactor : 1;
  return {
    x: Math.round(physical.x / f),
    y: Math.round(physical.y / f),
    width: Math.round(physical.width / f),
    height: Math.round(physical.height / f),
  };
}

/** A monitor's area in logical pixels, so it can be compared with a saved (logical) {@link Rect}. */
function monitorRect(m: Monitor): Rect {
  return toLogical({...m.position, ...m.size}, m.scaleFactor);
}

/** How much of a window must land on a monitor before we call it reachable. */
const MIN_VISIBLE_PX = 80;

function overlap(a: Rect, m: Rect): number {
  const right = Math.min(a.x + a.width, m.x + m.width);
  const left = Math.max(a.x, m.x);
  const bottom = Math.min(a.y + a.height, m.y + m.height);
  const top = Math.max(a.y, m.y);
  return Math.max(0, right - left) * Math.max(0, bottom - top);
}

/**
 * Keeps a restored window somewhere the user can actually reach it.
 *
 * Saved geometry outlives the display setup that produced it: unplug the second monitor, or dock a
 * laptop somewhere else, and a position that was perfectly good yesterday now puts the window off
 * every screen — visible nowhere, draggable never. So a saved rect is only honoured while a
 * meaningful part of it still lands on a monitor that exists *now*; otherwise it is nudged back
 * onto the primary one.
 *
 * Returns `undefined` when there is nothing to restore or no monitors were reported, which the
 * caller treats as "let the OS place it".
 */
export function clampToMonitors(saved: Rect | undefined, monitors: Monitor[]): Rect | undefined {
  if (!saved || monitors.length === 0) return undefined;
  // Compared in logical pixels, like `saved` — monitors are reported physical.
  const screens = monitors.map(monitorRect);

  const needed = Math.min(MIN_VISIBLE_PX * MIN_VISIBLE_PX, saved.width * saved.height);
  if (screens.some((m) => overlap(saved, m) >= needed)) return saved;

  // Off every current screen — put it back on the first monitor, keeping its size where that fits.
  const home = screens[0]!;
  const width = Math.min(saved.width, home.width);
  const height = Math.min(saved.height, home.height);

  return {
    width,
    height,
    x: home.x + Math.max(0, Math.round((home.width - width) / 2)),
    y: home.y + Math.max(0, Math.round((home.height - height) / 2)),
  };
}

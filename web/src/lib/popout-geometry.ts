/**
 * Pure geometry maths for pop-out windows (#349), kept apart from the Tauri calls so it can be
 * tested without a window.
 */

export type Rect = {x: number; y: number; width: number; height: number};

/** A monitor as Tauri reports it: a position and a size, in physical pixels. */
export type Monitor = {position: {x: number; y: number}; size: {width: number; height: number}};

/** How much of a window must land on a monitor before we call it reachable. */
const MIN_VISIBLE_PX = 80;

function overlap(a: Rect, m: Monitor): number {
  const right = Math.min(a.x + a.width, m.position.x + m.size.width);
  const left = Math.max(a.x, m.position.x);
  const bottom = Math.min(a.y + a.height, m.position.y + m.size.height);
  const top = Math.max(a.y, m.position.y);
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

  const needed = Math.min(MIN_VISIBLE_PX * MIN_VISIBLE_PX, saved.width * saved.height);
  if (monitors.some((m) => overlap(saved, m) >= needed)) return saved;

  // Off every current screen — put it back on the first monitor, keeping its size where that fits.
  const home = monitors[0]!;
  const width = Math.min(saved.width, home.size.width);
  const height = Math.min(saved.height, home.size.height);

  return {
    width,
    height,
    x: home.position.x + Math.max(0, Math.round((home.size.width - width) / 2)),
    y: home.position.y + Math.max(0, Math.round((home.size.height - height) / 2)),
  };
}

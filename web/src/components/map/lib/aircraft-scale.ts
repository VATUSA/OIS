/**
 * Zoom-dependent aircraft icon scaling, ported from VATSIM Radar
 * (`app/utils/map/aircraft-scale.ts` `getZoomScaleMultiplier`).
 *
 * VATSIM Radar sizes airborne glyphs on a hand-tuned, piecewise-linear curve of the map zoom rather
 * than a constant pixel size: planes are small when zoomed out (so an en-route view isn't a wall of
 * icons) and grow as you zoom in. We reproduce the same curve shape and normalize it so a glyph is its
 * configured base size at `REF_ZOOM`, then apply the multiplier to deck's per-icon `getSize`.
 */

// VATSIM Radar's airborne heuristic anchors (their MAX_MAP_ZOOM is 20).
const MIN_ZOOM = 2;
const BASELINE_ZOOM = 18.5;
const MAX_ZOOM = 20;
const MIN_MULT = 0.55;
const BASELINE_MULT = 2;
const MAX_MULT = 6;

/** Zoom at which the multiplier is 1 (glyph renders at its base pixel size). */
const REF_ZOOM = 6;

/** VATSIM Radar's raw airborne multiplier at a given zoom (piecewise-linear, ~0.55 → 2 → 6). */
function heuristicAtZoom(zoom: number): number {
  const z = Math.min(Math.max(zoom, MIN_ZOOM), MAX_ZOOM);
  if (z <= BASELINE_ZOOM) {
    const ratio = (z - MIN_ZOOM) / (BASELINE_ZOOM - MIN_ZOOM);
    return MIN_MULT + (BASELINE_MULT - MIN_MULT) * ratio;
  }
  const ratio = (z - BASELINE_ZOOM) / (MAX_ZOOM - BASELINE_ZOOM);
  return BASELINE_MULT + (MAX_MULT - BASELINE_MULT) * ratio;
}

const REF_MULT = heuristicAtZoom(REF_ZOOM);

/**
 * Multiplier to apply to an aircraft glyph's base pixel size for the current map zoom. ≈1 at REF_ZOOM,
 * <1 when zoomed out, >1 when zoomed in. Returns 1 for a non-finite zoom (safety before the first view
 * event).
 */
export function zoomAircraftScale(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return heuristicAtZoom(zoom) / REF_MULT;
}

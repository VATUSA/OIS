/** Class strings for the controls and panels floating over a map (DESIGN.md: panel surface, hairline, no shadow). */

/** A floating panel: legend, scrubber, hint bar. */
export const MAP_PANEL = "rounded-md border border-line bg-panel";

/** A pill button over the map (Home, ATC, layer toggles). */
export const MAP_BUTTON =
  "inline-flex h-8 items-center gap-1.5 rounded-full border border-line bg-panel px-3 text-xs font-semibold text-ink-2 transition-colors hover:bg-panel-2 hover:text-ink [&_svg]:size-3.5";

/** Added to `MAP_BUTTON` while its toggle is on. */
export const MAP_BUTTON_ON = "border-brand/40 bg-brand-soft text-ink hover:bg-brand-soft";

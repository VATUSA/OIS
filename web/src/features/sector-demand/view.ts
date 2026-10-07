import type {SectorGridRow} from "@ois/ui";

import type {SectorDemandRow} from "./sector-demand";

/** Which of an ARTCC's two tables (#725): Low/High/Ultra High, or Approach Control. */
export type DemandTableKind = "enroute" | "tracon";

/**
 * One table's controls (#725): how many hours to draw, and whether to hide sectors that stay green
 * over the next `alertSpanH` hours. The span is independent of the range — a 4-hour table may filter on
 * the next 1.5 hours.
 */
export type DemandView = { rangeH: number; alertOnly: boolean; alertSpanH: number };

/** Every bin is a quarter-hour; both controls move in whole bins. */
export const BIN_H = 0.25;
export const RANGE_MIN_H = 2;
export const RANGE_MAX_H = 6;
export const SPAN_MIN_H = BIN_H;
export const SPAN_MAX_H = 6;

/**
 * The 2–6 h range defaults to 4 h, and the filter starts on at a 2 h span (owner decision, #725): a
 * table opens on the sectors that need attention soon. Switching it off is remembered like any other
 * control, per browser per facility per table.
 */
export const DEFAULT_VIEW: DemandView = { rangeH: 4, alertOnly: true, alertSpanH: 2 };

/** The filter's span choices, one per quarter-hour up to six hours. */
export const SPAN_CHOICES_H: readonly number[] = Array.from(
  { length: Math.round((SPAN_MAX_H - SPAN_MIN_H) / BIN_H) + 1 },
  (_, i) => SPAN_MIN_H + i * BIN_H,
);

/** `2.00 h` — the form the issue's own messages use. */
export const formatHours = (h: number) => `${h.toFixed(2)} h`;

/** How many bins `hours` covers. */
export const binsFor = (hours: number) => Math.round(hours / BIN_H);

/** Clamps `h` into `[min, max]` on whole bins; anything not a finite number reads as `fallback`. */
function snap(h: unknown, min: number, max: number, fallback: number): number {
  if (typeof h !== "number" || !Number.isFinite(h)) return fallback;
  return Math.min(max, Math.max(min, Math.round(h / BIN_H) * BIN_H));
}

/** Whether a row has a non-green bin among its first `spanH` hours. */
export function isAlerting(row: SectorDemandRow, spanH: number): boolean {
  return row.bins.slice(0, binsFor(spanH)).some((b) => b.level !== "ok");
}

/**
 * The rows and bins a table draws under `view`: the filter is judged on its own span over all six
 * computed hours, then every row is cut to the drawn range. Pure slicing of what the server sent — the
 * range never refetches.
 */
export function visibleGrid(
  rows: readonly SectorDemandRow[],
  binStartsMs: readonly number[],
  view: DemandView,
): { rows: SectorGridRow[]; binStarts: number[] } {
  const n = binsFor(view.rangeH);
  const kept = view.alertOnly ? rows.filter((r) => isAlerting(r, view.alertSpanH)) : rows;
  return {
    binStarts: binStartsMs.slice(0, n),
    rows: kept.map((r) => ({
      id: r.sector_id,
      name: r.name ?? undefined,
      limit: r.limit,
      carries: r.consolidated,
      cells: r.bins.slice(0, n).map((b) => ({ combined: b.combined, active: b.active, level: b.level })),
    })),
  };
}

// --- Per-browser memory (#725: both controls, and a neighbour's open state, per facility) ---------
//
// View state, not a synced setting, so it lives in this browser's localStorage. Storage can be
// missing or throw (private mode, blocked site data), and the page must work without it, so every
// access is guarded and a bad value reads as the default.

const viewKey = (artcc: string, kind: DemandTableKind) => `ois.sectorDemand.view.${artcc}.${kind}`;
const openKey = (pageArtcc: string, neighbour: string) => `ois.sectorDemand.open.${pageArtcc}.${neighbour}`;

/** The remembered controls of one ARTCC's table, or the defaults. */
export function loadView(artcc: string, kind: DemandTableKind): DemandView {
  try {
    const raw = localStorage.getItem(viewKey(artcc, kind));
    if (!raw) return DEFAULT_VIEW;
    const v = JSON.parse(raw) as Partial<Record<keyof DemandView, unknown>>;
    return {
      rangeH: snap(v.rangeH, RANGE_MIN_H, RANGE_MAX_H, DEFAULT_VIEW.rangeH),
      alertOnly: typeof v.alertOnly === "boolean" ? v.alertOnly : DEFAULT_VIEW.alertOnly,
      alertSpanH: snap(v.alertSpanH, SPAN_MIN_H, SPAN_MAX_H, DEFAULT_VIEW.alertSpanH),
    };
  } catch {
    return DEFAULT_VIEW;
  }
}

export function saveView(artcc: string, kind: DemandTableKind, view: DemandView): void {
  try {
    localStorage.setItem(viewKey(artcc, kind), JSON.stringify(view));
  } catch {
    /* non-fatal: the view just isn't remembered */
  }
}

/** Whether `neighbour`'s table was left open on `pageArtcc`'s page. Collapsed unless it was. */
export function loadOpen(pageArtcc: string, neighbour: string): boolean {
  try {
    return localStorage.getItem(openKey(pageArtcc, neighbour)) === "1";
  } catch {
    return false;
  }
}

export function saveOpen(pageArtcc: string, neighbour: string, open: boolean): void {
  try {
    localStorage.setItem(openKey(pageArtcc, neighbour), open ? "1" : "0");
  } catch {
    /* non-fatal */
  }
}

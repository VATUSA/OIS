// The customizable "My dashboard" data model. The whole object is persisted per user as an
// opaque jsonb blob (namespace "dashboard", see web/src/lib/preferences.ts). `layout` is the
// react-grid-layout geometry; `layout[i].i === widget.id` ties a cell to its widget.
//
// Phase 1 ships the "view", "stat", and "map" kinds. "table" and "chart" arrive in later phases,
// so keep this union open for extension and tolerate unknown kinds when reading stored state.

export type ViewId =
  | "departures"
  | "taxi"
  | "airport-summary"
  | "airport-aircraft"
  | "airport-ladder"
  | "airport-demand";

export type StatMetricId =
  | "pilots"
  | "programs"
  | "tmis"
  | "ground-stops"
  | "gdps"
  | "fcas";

/** Include-filters for the arrival-ladder widget. Each list is an allow-list; empty/undefined = all. */
export interface LadderFilters {
  /** Arrival gates (STAR group names, e.g. "ROBUC") to keep. */
  gates?: string[];
  /** Flight statuses to keep: "airborne" | "ground" | "proposed". */
  statuses?: string[];
  /** Departure-airport prefixes (uppercased) to keep, matched against each flight's origin. */
  origins?: string[];
  /** Aircraft-type prefixes (uppercased) to keep — e.g. ["B73","A32"] for a jets-only view. */
  types?: string[];
}

export interface ViewWidget {
  id: string;
  kind: "view";
  title?: string;
  view: ViewId;
  /** The airport this view is bound to (all Phase-1 views are per-airport). */
  icao: string;
  /** Arrival-ladder filters (only meaningful when `view` is "airport-ladder"). */
  filters?: LadderFilters;
}

export interface StatWidget {
  id: string;
  kind: "stat";
  title?: string;
  metric: StatMetricId;
}

export interface MapWidget {
  id: string;
  kind: "map";
  title?: string;
  initialFlight?: string;
}

/** A dashboard scope of "a whole ATC facility" — an ARTCC (center) or TRACON (approach). Resolved to
 * its member airports at render (see `lib/facilities`), so membership stays current. */
export interface FacilityRef {
  kind: "artcc" | "tracon";
  id: string;
}

export interface TableWidget {
  id: string;
  kind: "table";
  title?: string;
  /** DataSource id from the registry (features/dashboard/sources). */
  source: string;
  params?: { icao?: string; facility?: FacilityRef };
  /** Visible column keys (field keys), in order. Undefined = all of the source's fields. */
  columns?: string[];
  /** TanStack Table sorting state, persisted. */
  sort?: { id: string; desc: boolean }[];
}

export type ChartAggregate = "none" | "count" | "sum" | "avg" | "min" | "max";

/** A horizontal reference line drawn at `value`, in the chart's actual units. */
export interface ChartThreshold {
  value: number;
  color: string;
}

export interface ChartWidget {
  id: string;
  kind: "chart";
  title?: string;
  /** DataSource id from the registry (features/dashboard/sources). */
  source: string;
  /** icaos powers multi-airport comparison; icao kept for back-compat / single-airport;
   * facility expands to its member airports at render. */
  params?: { icao?: string; icaos?: string[]; facility?: FacilityRef };
  chartType: "bar" | "line" | "area" | "scatter" | "pie";
  /** Field key for the x axis (category/time), or "__airport" to compare airports directly. */
  x: string;
  /** One or more numeric field keys plotted as series. */
  y: string[];
  /**
   * How to shape the data. "none" plots raw rows; otherwise rows are grouped by x and the y
   * series are aggregated ("count" ignores y and counts rows per x group).
   */
  aggregate?: ChartAggregate;
  /** Keep only the top N groups by value (0/undefined = all). Applied when aggregated. */
  topN?: number;
  /** Per-series color overrides, keyed by series key (field key or airport icao). */
  colors?: Record<string, string>;
  /** Rescale each series to 0–100% of its own max, so mixed-scale series compare on one axis. */
  normalize?: boolean;
  /** Horizontal reference lines (ignored while normalized, since units differ). */
  thresholds?: ChartThreshold[];
}

/** A free-text / heading block for titling and annotating a board (Grafana "text panel"). */
export interface TextWidget {
  id: string;
  kind: "text";
  content: string;
  /** Font scale. */
  size?: "sm" | "md" | "lg" | "xl";
  align?: "left" | "center" | "right";
}

/** A section divider — a thin rule spanning its cell, optionally labelled. */
export interface DividerWidget {
  id: string;
  kind: "divider";
  orientation: "horizontal" | "vertical";
  /** Optional caption shown on a horizontal divider. */
  label?: string;
}

/** Online ATC positions for a whole facility (its center/approach positions + its airports' towers). */
export interface AtcWidget {
  id: string;
  kind: "atc";
  title?: string;
  facility: FacilityRef;
}

export type Widget =
  | ViewWidget
  | StatWidget
  | MapWidget
  | TableWidget
  | ChartWidget
  | AtcWidget
  | TextWidget
  | DividerWidget;
export type WidgetKind = Widget["kind"];

/** react-grid-layout cell geometry (grid units). `i` matches the widget id. */
export interface GridCell {
  i: string;
  x: number;
  y: number;
  w: number;
  h: number;
  minW?: number;
  minH?: number;
}

export interface DashboardState {
  version: 1;
  widgets: Widget[];
  layout: GridCell[];
}

export const EMPTY_DASHBOARD: DashboardState = {
  version: 1,
  widgets: [],
  layout: [],
};

/** Grid columns at the widest breakpoint — the geometry all defaults are expressed in. */
export const GRID_COLS = 12;

/** Sensible default cell size per widget, in grid units (w of 12, h in rowHeight steps). */
export function defaultCell(w: Widget): { w: number; h: number; minW: number; minH: number } {
  switch (w.kind) {
    case "stat":
      return { w: 3, h: 2, minW: 2, minH: 2 };
    case "map":
      return { w: 6, h: 5, minW: 4, minH: 3 };
    case "table":
      return { w: 6, h: 4, minW: 3, minH: 3 };
    case "chart":
      return { w: 5, h: 4, minW: 3, minH: 3 };
    case "atc":
      return { w: 4, h: 6, minW: 3, minH: 3 };
    case "text":
      return { w: 4, h: 1, minW: 2, minH: 1 };
    case "divider":
      return w.orientation === "vertical"
        ? { w: 1, h: 4, minW: 1, minH: 2 }
        : { w: 12, h: 1, minW: 2, minH: 1 };
    case "view":
    default:
      return { w: 6, h: 5, minW: 3, minH: 3 };
  }
}

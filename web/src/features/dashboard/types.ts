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

export interface ViewWidget {
  id: string;
  kind: "view";
  title?: string;
  view: ViewId;
  /** The airport this view is bound to (all Phase-1 views are per-airport). */
  icao: string;
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

export interface TableWidget {
  id: string;
  kind: "table";
  title?: string;
  /** DataSource id from the registry (features/dashboard/sources). */
  source: string;
  params?: { icao?: string };
  /** Visible column keys (field keys), in order. Undefined = all of the source's fields. */
  columns?: string[];
  /** TanStack Table sorting state, persisted. */
  sort?: { id: string; desc: boolean }[];
}

export type ChartAggregate = "none" | "count" | "sum" | "avg" | "min" | "max";

export interface ChartWidget {
  id: string;
  kind: "chart";
  title?: string;
  /** DataSource id from the registry (features/dashboard/sources). */
  source: string;
  params?: { icao?: string };
  chartType: "bar" | "line" | "area";
  /** Field key for the x axis (category/time). */
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
}

export type Widget = ViewWidget | StatWidget | MapWidget | TableWidget | ChartWidget;
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

/** Sensible default cell size per kind, in grid units (w of 12, h in rowHeight steps). */
export function defaultCell(kind: WidgetKind): { w: number; h: number; minW: number; minH: number } {
  switch (kind) {
    case "stat":
      return { w: 3, h: 2, minW: 2, minH: 2 };
    case "map":
      return { w: 6, h: 5, minW: 4, minH: 3 };
    case "table":
      return { w: 6, h: 4, minW: 3, minH: 3 };
    case "chart":
      return { w: 5, h: 4, minW: 3, minH: 3 };
    case "view":
    default:
      return { w: 6, h: 5, minW: 3, minH: 3 };
  }
}

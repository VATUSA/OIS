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

export type Widget = ViewWidget | StatWidget | MapWidget;
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
    case "view":
    default:
      return { w: 6, h: 5, minW: 3, minH: 3 };
  }
}

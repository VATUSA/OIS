// Shared chart constants/helpers, imported by both chart-widget.tsx and chart-config-panel.tsx.

import {AIRPORT_KEY, type DataSource} from "./sources";
import type {ChartAggregate} from "./types";

export const CHART_TYPES = ["line", "area", "bar", "scatter", "pie"] as const;
export type ChartType = (typeof CHART_TYPES)[number];

export const AGGREGATES: { id: ChartAggregate; label: string }[] = [
  { id: "count", label: "Count" },
  { id: "sum", label: "Sum" },
  { id: "avg", label: "Average" },
  { id: "min", label: "Min" },
  { id: "max", label: "Max" },
  { id: "none", label: "Raw rows" },
];

export const TOP_OPTIONS = [0, 5, 10, 15, 20, 30];

export interface Series {
  key: string;
  label: string;
}

export const labelOf = (source: DataSource, key: string) =>
  key === AIRPORT_KEY ? "Airport" : (source.fields.find((f) => f.key === key)?.label ?? key);

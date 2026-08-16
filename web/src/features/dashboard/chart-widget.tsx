import {useEffect, useMemo, useRef, useState} from "react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@ois/ui";
import {areaY, barY, type ChartValue, defineChart, lineY} from "@tanstack/charts";
import {Chart} from "@tanstack/react-charts";
import {scaleBand, scaleLinear, scalePoint} from "d3-scale";
import {Check} from "lucide-react";

import {type DataSource, DATA_SOURCES_BY_ID, type Row} from "./sources";
import type {ChartAggregate, ChartWidget as ChartWidgetT} from "./types";

const CHART_TYPES = ["line", "area", "bar"] as const;
type ChartType = (typeof CHART_TYPES)[number];

const AGGREGATES: { id: ChartAggregate; label: string }[] = [
  { id: "count", label: "Count" },
  { id: "sum", label: "Sum" },
  { id: "avg", label: "Average" },
  { id: "min", label: "Min" },
  { id: "max", label: "Max" },
  { id: "none", label: "Raw rows" },
];

const TOP_OPTIONS = [0, 5, 10, 15, 20, 30];
const COUNT_KEY = "__count";

/** Categorical series palette, tuned for the dark card background. */
const PALETTE = [
  "#60a5fa",
  "#34d399",
  "#f59e0b",
  "#f472b6",
  "#a78bfa",
  "#f87171",
  "#22d3ee",
  "#a3e635",
];
const colorAt = (i: number) => PALETTE[i % PALETTE.length];

const compactFmt = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
/** Compact axis numbers: 40000 → "40K", 1_500_000 → "1.5M", small values as-is. */
function fmtNumber(v: number): string {
  if (!Number.isFinite(v)) return "";
  return Math.abs(v) >= 1000 ? compactFmt.format(v) : String(Math.round(v * 100) / 100);
}

interface Series {
  key: string;
  label: string;
}

/** A sensible line-first, aggregated starting config for a chart bound to a source. */
export function defaultChartConfig(source: DataSource): {
  chartType: ChartType;
  x: string;
  y: string[];
  aggregate: ChartAggregate;
  topN: number;
} {
  const nums = source.fields.filter((f) => f.type === "number");
  const cat = source.fields.find((f) => f.type === "string" || f.type === "time");
  return {
    chartType: "line",
    x: (cat ?? source.fields[0])?.key ?? "",
    y: nums.slice(0, 1).map((f) => f.key),
    aggregate: "count",
    topN: 15,
  };
}

function useSize() {
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  return [ref, size] as const;
}

const xAccessor = (key: string) => (d: Row): ChartValue => {
  const v = d[key];
  if (typeof v === "number" || v instanceof Date) return v;
  return String(v ?? "");
};

const yAccessor = (key: string) => (d: Row): number => {
  const n = Number(d[key]);
  return Number.isFinite(n) ? n : 0;
};

function reduce(agg: ChartAggregate, vals: number[]): number {
  if (vals.length === 0) return 0;
  switch (agg) {
    case "sum":
      return vals.reduce((a, b) => a + b, 0);
    case "avg":
      return Math.round((vals.reduce((a, b) => a + b, 0) / vals.length) * 100) / 100;
    case "min":
      return Math.min(...vals);
    case "max":
      return Math.max(...vals);
    default:
      return vals.length;
  }
}

const labelOf = (source: DataSource, key: string) =>
  source.fields.find((f) => f.key === key)?.label ?? key;

/**
 * Shape rows for the chart. "none" plots the raw rows; any other aggregate groups by x and either
 * counts rows or reduces each y series. Aggregated data is trimmed to the top-N groups by value
 * (bars stay value-ranked; lines/areas re-sort by x so the trend reads left-to-right).
 */
function shapeChartData(
  rows: Row[],
  source: DataSource,
  xKey: string,
  yKeys: string[],
  aggregate: ChartAggregate,
  chartType: ChartType,
  topN: number | undefined,
): { data: Row[]; series: Series[]; categorical: boolean } {
  if (aggregate === "none") {
    const series = yKeys.map((k) => ({ key: k, label: labelOf(source, k) }));
    const xType = source.fields.find((f) => f.key === xKey)?.type;
    return { data: rows, series, categorical: xType !== "number" };
  }

  const groups = new Map<string, Row[]>();
  for (const r of rows) {
    const k = String(r[xKey] ?? "");
    let arr = groups.get(k);
    if (!arr) groups.set(k, (arr = []));
    arr.push(r);
  }

  let series: Series[];
  let data: Row[] = [];
  if (aggregate === "count") {
    series = [{ key: COUNT_KEY, label: "Count" }];
    for (const [k, rs] of groups) data.push({ [xKey]: k, [COUNT_KEY]: rs.length });
  } else {
    series = yKeys.map((k) => ({ key: k, label: labelOf(source, k) }));
    for (const [k, rs] of groups) {
      const row: Row = { [xKey]: k };
      for (const y of yKeys) {
        const vals = rs.map((r) => Number(r[y])).filter((v) => Number.isFinite(v));
        row[y] = reduce(aggregate, vals);
      }
      data.push(row);
    }
  }

  const sortKey = series[0]?.key ?? COUNT_KEY;
  data.sort((a, b) => Number(b[sortKey]) - Number(a[sortKey]));
  if (topN && topN > 0) data = data.slice(0, topN);
  if (chartType !== "bar") {
    data.sort((a, b) =>
      String(a[xKey]).localeCompare(String(b[xKey]), undefined, { numeric: true }),
    );
  }
  return { data, series, categorical: true };
}

function buildDefinition(
  data: Row[],
  chartType: ChartType,
  xKey: string,
  xLabel: string,
  categorical: boolean,
  series: Series[],
) {
  const x = xAccessor(xKey);
  const marks = series.map((s, i) => {
    const y = yAccessor(s.key);
    const color = colorAt(i);
    if (chartType === "line") return lineY(data, { x, y, stroke: color });
    if (chartType === "area") return areaY(data, { x, y, fill: color });
    return barY(data, { x, y, fill: color });
  });
  const xScale = categorical ? (chartType === "bar" ? scaleBand : scalePoint) : scaleLinear;
  // A single-series chart labels its y axis with that series; multi-series relies on the legend.
  const yLabel = series.length === 1 ? series[0].label : undefined;
  return defineChart({
    marks,
    x: { scale: xScale, axis: { label: xLabel } },
    y: {
      scale: scaleLinear,
      axis: { label: yLabel, ticks: { format: (v) => fmtNumber(Number(v)) } },
    },
  });
}

function Picker({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="secondary" className="h-7">
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-[50vh] w-44 overflow-y-auto">
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CheckItem({
  checked,
  label,
  onSelect,
  keepOpen,
}: {
  checked: boolean;
  label: string;
  onSelect: () => void;
  keepOpen?: boolean;
}) {
  return (
    <DropdownMenuItem
      onSelect={(e) => {
        if (keepOpen) e.preventDefault();
        onSelect();
      }}
    >
      <Check className={"size-3.5 " + (checked ? "opacity-100" : "opacity-0")} />
      {label}
    </DropdownMenuItem>
  );
}

function ConfigBar({
  source,
  widget,
  onChange,
}: {
  source: DataSource;
  widget: ChartWidgetT;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const set = (patch: Record<string, unknown>) => onChange(widget.id, patch);
  const nums = source.fields.filter((f) => f.type === "number");
  const aggregate = widget.aggregate ?? "none";
  const aggLabel = AGGREGATES.find((a) => a.id === aggregate)?.label ?? "Count";
  const xLabel = source.fields.find((f) => f.key === widget.x)?.label ?? "X";

  const ySet = new Set(widget.y);
  const toggleY = (key: string) => {
    const next = nums.map((f) => f.key).filter((k) => (k === key ? !ySet.has(k) : ySet.has(k)));
    if (next.length > 0) set({ y: next });
  };

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Picker label={`Type · ${widget.chartType}`}>
        {CHART_TYPES.map((t) => (
          <CheckItem
            key={t}
            checked={widget.chartType === t}
            label={t}
            onSelect={() => set({ chartType: t })}
          />
        ))}
      </Picker>
      <Picker label={`Group by · ${xLabel}`}>
        <DropdownMenuLabel>X axis / group</DropdownMenuLabel>
        {source.fields.map((f) => (
          <CheckItem
            key={f.key}
            checked={widget.x === f.key}
            label={f.label}
            onSelect={() => set({ x: f.key })}
          />
        ))}
      </Picker>
      <Picker label={`Agg · ${aggLabel}`}>
        <DropdownMenuLabel>Aggregate</DropdownMenuLabel>
        {AGGREGATES.map((a) => (
          <CheckItem
            key={a.id}
            checked={aggregate === a.id}
            label={a.label}
            onSelect={() => set({ aggregate: a.id })}
          />
        ))}
      </Picker>
      {aggregate !== "count" && (
        <Picker label={`Y · ${widget.y.length}`}>
          <DropdownMenuLabel>Y series (numeric)</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {nums.map((f) => (
            <CheckItem
              key={f.key}
              checked={ySet.has(f.key)}
              label={f.label}
              onSelect={() => toggleY(f.key)}
              keepOpen
            />
          ))}
        </Picker>
      )}
      {aggregate !== "none" && (
        <Picker label={`Top · ${widget.topN ? widget.topN : "all"}`}>
          <DropdownMenuLabel>Limit</DropdownMenuLabel>
          {TOP_OPTIONS.map((n) => (
            <CheckItem
              key={n}
              checked={(widget.topN ?? 0) === n}
              label={n === 0 ? "All" : `Top ${n}`}
              onSelect={() => set({ topN: n })}
            />
          ))}
        </Picker>
      )}
    </div>
  );
}

function ChartInner({
  source,
  widget,
  editing,
  onChange,
}: {
  source: DataSource;
  widget: ChartWidgetT;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const { rows, isLoading, isError } = source.useRows(widget.params ?? {});
  const [ref, size] = useSize();
  const aggregate = widget.aggregate ?? "none";

  const shaped = useMemo(
    () => shapeChartData(rows, source, widget.x, widget.y, aggregate, widget.chartType, widget.topN),
    [rows, source, widget.x, widget.y, aggregate, widget.chartType, widget.topN],
  );
  const xLabel = labelOf(source, widget.x);
  const definition = useMemo(
    () =>
      buildDefinition(
        shaped.data,
        widget.chartType,
        widget.x,
        xLabel,
        shaped.categorical,
        shaped.series,
      ),
    [shaped, widget.chartType, widget.x, xLabel],
  );

  const ready = widget.x && (aggregate === "count" || widget.y.length > 0);
  const empty = shaped.data.length === 0;
  const showChart = ready && !isError && !empty;

  return (
    <div className="flex h-full flex-col gap-2 p-2">
      {editing && <ConfigBar source={source} widget={widget} onChange={onChange} />}
      <div ref={ref} className="min-h-0 flex-1">
        {isError ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">Couldn&apos;t load data.</p>
        ) : !ready ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">
            Pick a group-by field, and a Y series unless counting.
          </p>
        ) : isLoading && empty ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">Loading…</p>
        ) : empty ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">No data.</p>
        ) : size.w > 0 && size.h > 0 ? (
          <Chart
            definition={definition}
            ariaLabel={source.label}
            width={size.w}
            height={size.h}
          />
        ) : null}
      </div>
      {showChart && shaped.series.length > 1 && <Legend series={shaped.series} />}
    </div>
  );
}

function Legend({ series }: { series: Series[] }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-1 text-xs text-muted-foreground">
      {series.map((s, i) => (
        <span key={s.key} className="inline-flex items-center gap-1.5">
          <span
            className="inline-block size-2.5 rounded-sm"
            style={{ background: colorAt(i) }}
          />
          {s.label}
        </span>
      ))}
    </div>
  );
}

export function ChartWidget({
  widget,
  editing,
  onChange,
}: {
  widget: ChartWidgetT;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const source = DATA_SOURCES_BY_ID[widget.source];
  if (!source) {
    return <div className="p-4 text-sm text-muted-foreground">Unknown data source.</div>;
  }
  return (
    <ChartInner key={source.id} source={source} widget={widget} editing={editing} onChange={onChange} />
  );
}

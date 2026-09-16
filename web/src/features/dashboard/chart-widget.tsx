import {useMemo, useState} from "react";
import {Button, ChartTooltip, useElementSize, Donut, EmptyState, formatCompact, useChartTheme} from "@ois/ui";
import {areaY, barY, type ChartValue, defineChart, dot, lineY, ruleY} from "@tanstack/charts";
import {tooltip} from "@tanstack/charts/tooltip";
// The /tooltip entry is the same Chart with the built-in hover crosshair/focus enabled.
import {Chart} from "@tanstack/react-charts/tooltip";
import {scaleBand, scaleLinear, scalePoint} from "d3-scale";
import {Settings2} from "lucide-react";

import {ChartConfigPanel} from "./chart-config-panel";
import {type ChartType, labelOf, type Series} from "./chart-shared";
import {facilityAirports, useFacilityDirectory} from "@/lib/facilities";
import {AIRPORT_KEY, type DataSource, DATA_SOURCES_BY_ID, type Row} from "./sources";
import type {ChartAggregate, ChartThreshold, ChartWidget as ChartWidgetT} from "./types";
import {useReportWidgetStatus} from "./widget-status";

const COUNT_KEY = "__count";

/** A sensible line-first, aggregated starting config for a chart bound to a source. */
export function defaultChartConfig(source: DataSource): {
  chartType: ChartType;
  x: string;
  y: string[];
  aggregate: ChartAggregate;
  topN: number;
} {
  const nums = source.fields.filter((f) => f.type === "number");
  const preferred = ["status", "phase", "gate", "mode", "artcc", "dep", "arrival", "arr", "icao"];
  const cat =
    source.fields.find((f) => preferred.includes(f.key)) ??
    source.fields.find((f) => f.type === "string" || f.type === "time");
  return {
    chartType: "line",
    x: (cat ?? source.fields[0])?.key ?? "",
    y: nums.slice(0, 1).map((f) => f.key),
    aggregate: "count",
    topN: 15,
  };
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

interface Shaped {
  data: Row[];
  series: Series[];
  categorical: boolean;
}

/** Rescale each series to 0–100% of its own max, so mixed-scale series compare on one axis. */
function normalizeShaped(s: Shaped): Shaped {
  const data = s.data.map((r) => ({ ...r }));
  for (const ser of s.series) {
    let max = 0;
    for (const r of data) {
      const v = Math.abs(Number(r[ser.key]) || 0);
      if (v > max) max = v;
    }
    if (max > 0) {
      for (const r of data) {
        r[ser.key] = Math.round(((Number(r[ser.key]) || 0) / max) * 1000) / 10;
      }
    }
  }
  return { ...s, data };
}

function shapeChartData(
  rows: Row[],
  source: DataSource,
  xKey: string,
  yKeys: string[],
  aggregate: ChartAggregate,
  chartType: ChartType,
  topN: number | undefined,
  splitKey: string | undefined,
  normalize: boolean,
): Shaped {
  const finish = (s: Shaped): Shaped => (normalize ? normalizeShaped(s) : s);

  if (aggregate === "none") {
    const series = yKeys.map((k) => ({ key: k, label: labelOf(source, k) }));
    const xType = source.fields.find((f) => f.key === xKey)?.type;
    return finish({ data: rows, series, categorical: xType !== "number" });
  }

  const orderXThenTrim = (data: Row[], keys: string[]): Row[] => {
    const total = (r: Row) => keys.reduce((s, k) => s + (Number(r[k]) || 0), 0);
    data.sort((a, b) => total(b) - total(a));
    let trimmed = topN && topN > 0 ? data.slice(0, topN) : data;
    if (chartType !== "bar") {
      trimmed = [...trimmed].sort((a, b) =>
        String(a[xKey]).localeCompare(String(b[xKey]), undefined, { numeric: true }),
      );
    }
    return trimmed;
  };

  // Compare-airports pivot: series = airports.
  if (splitKey && splitKey !== xKey) {
    const yKey = aggregate === "count" ? null : yKeys[0];
    const splitVals: string[] = [];
    const seen = new Set<string>();
    const xMap = new Map<string, Map<string, Row[]>>();
    for (const r of rows) {
      const xv = String(r[xKey] ?? "");
      const sv = String(r[splitKey] ?? "");
      if (!seen.has(sv)) {
        seen.add(sv);
        splitVals.push(sv);
      }
      let m = xMap.get(xv);
      if (!m) xMap.set(xv, (m = new Map()));
      let arr = m.get(sv);
      if (!arr) m.set(sv, (arr = []));
      arr.push(r);
    }
    const series = splitVals.map((s) => ({ key: s, label: s }));
    const data: Row[] = [];
    for (const [xv, m] of xMap) {
      const row: Row = { [xKey]: xv };
      for (const s of splitVals) {
        const rs = m.get(s) ?? [];
        row[s] =
          yKey == null
            ? rs.length
            : reduce(aggregate, rs.map((r) => Number(r[yKey])).filter((v) => Number.isFinite(v)));
      }
      data.push(row);
    }
    return finish({ data: orderXThenTrim(data, splitVals), series, categorical: true });
  }

  // Single-dimension: group by x, series = y fields (or Count).
  const groups = new Map<string, Row[]>();
  for (const r of rows) {
    const k = String(r[xKey] ?? "");
    let arr = groups.get(k);
    if (!arr) groups.set(k, (arr = []));
    arr.push(r);
  }
  let series: Series[];
  const data: Row[] = [];
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
  return finish({ data: orderXThenTrim(data, series.map((s) => s.key)), series, categorical: true });
}

function buildDefinition(
  data: Row[],
  chartType: ChartType,
  xKey: string,
  xLabel: string,
  categorical: boolean,
  series: Series[],
  colorFor: (key: string, i: number) => string,
  categoryColors: Record<string, string>,
  normalized: boolean,
  thresholds: ChartThreshold[],
  theme: ReturnType<typeof useChartTheme>["theme"],
) {
  const x = xAccessor(xKey);
  // Bar/scatter render one discrete mark per datum, so a category override can recolor a single
  // column regardless of which series it belongs to; line/area draw one continuous shape per
  // series, where per-point recoloring is a materially different (and more ambiguous) visual, so
  // they keep the flat per-series color.
  const resolvedColor = (d: Row, fallback: string) => categoryColors[String(x(d) ?? "")] ?? fallback;
  const seriesMarks = series.flatMap((s, i) => {
    const y = yAccessor(s.key);
    const color = colorFor(s.key, i);
    // `id: s.key` makes ChartPoint.markId (the tooltip's only per-point identity field) resolve
    // back to the real series — otherwise it defaults to an auto-generated id like "line-0" (#136).
    if (chartType === "line") return [lineY(data, { x, y, stroke: color, id: s.key })];
    if (chartType === "area") return [areaY(data, { x, y, fill: color, id: s.key })];
    if (chartType === "scatter") {
      // dot() only takes a static fill/stroke, unlike barY's per-datum channel — split into one
      // dot() mark per resolved color instead so a category override still recolors just its points.
      // All of a series' color-split marks share the same `id: s.key` — a correct many-to-one
      // mapping back to the series for the tooltip lookup.
      const groups = new Map<string, Row[]>();
      for (const d of data) {
        const c = resolvedColor(d, color);
        let arr = groups.get(c);
        if (!arr) groups.set(c, (arr = []));
        arr.push(d);
      }
      return [...groups.entries()].map(([c, gd]) => dot(gd, { x, y, fill: c, stroke: c, r: 3.5, id: s.key }));
    }
    return [barY(data, { x, y, fill: (d: Row) => resolvedColor(d, color), id: s.key })];
  });
  const thresholdMarks =
    !normalized && thresholds.length
      ? [
          ruleY(thresholds, {
            y: (t: ChartThreshold) => t.value,
            stroke: (t: ChartThreshold) => t.color,
            strokeDasharray: "5 4",
            strokeWidth: 1,
          }),
        ]
      : [];
  const xScale = categorical ? (chartType === "bar" ? scaleBand : scalePoint) : scaleLinear;
  const yLabel = normalized ? "% of max" : series.length === 1 ? series[0].label : undefined;
  const yFormat = normalized
    ? (v: ChartValue) => `${Math.round(Number(v))}%`
    : (v: ChartValue) => formatCompact(Number(v));
  return defineChart({
    marks: [...seriesMarks, ...thresholdMarks],
    scales: {
      x: { scale: xScale, axis: { label: xLabel } },
      y: { scale: scaleLinear, axis: { label: yLabel, ticks: { format: yFormat } } },
    },
    theme,
    tooltip,
  });
}

function Legend({
  series,
  colorFor,
  editable,
  onColor,
}: {
  series: Series[];
  colorFor: (key: string, i: number) => string;
  editable: boolean;
  onColor: (key: string, hex: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-1 text-xs text-ink-2">
      {series.map((s, i) => {
        const color = colorFor(s.key, i);
        return (
          <span key={s.key} className="inline-flex items-center gap-1.5">
            <span className="relative inline-block size-2.5 rounded-sm" style={{ background: color }}>
              {editable && (
                <input
                  type="color"
                  value={color}
                  title={`Colour for ${s.label}`}
                  onChange={(e) => onColor(s.key, e.target.value)}
                  className="absolute inset-0 size-full cursor-pointer opacity-0"
                />
              )}
            </span>
            {s.label}
          </span>
        );
      })}
    </div>
  );
}

/** Pie/share chart on the shared `Donut` (full pie). Slices = x groups; value = the first series. */
function PieChart({
  data,
  xKey,
  valueKey,
  size,
  colorFor,
  editable,
  onColor,
}: {
  data: Row[];
  xKey: string;
  valueKey: string;
  size: { w: number; h: number };
  colorFor: (key: string, i: number) => string;
  editable: boolean;
  onColor: (key: string, hex: string) => void;
}) {
  const slices = data
    .map((r) => ({ label: String(r[xKey] ?? ""), value: Math.max(0, Number(r[valueKey]) || 0) }))
    .filter((s) => s.value > 0);
  const total = slices.reduce((a, s) => a + s.value, 0);
  if (total <= 0) return <EmptyState>No data.</EmptyState>;

  const d = Math.max(60, Math.min(size.h, size.w * 0.62));
  const arcs = slices.map((s, i) => ({ label: s.label, value: s.value, frac: s.value / total, color: colorFor(s.label, i) }));

  return (
    <div className="flex h-full items-center gap-3 p-2">
      <Donut size={d} thickness={1} label="Share" slices={arcs.map((a) => ({ label: a.label, value: a.value, color: a.color }))} />
      <div className="flex max-h-full min-w-0 flex-col gap-1 overflow-auto text-xs">
        {arcs.map((a) => (
          <span key={a.label} className="inline-flex items-center gap-1.5">
            <span
              className="relative inline-block size-2.5 shrink-0 rounded-sm"
              style={{ background: a.color }}
            >
              {editable && (
                <input
                  type="color"
                  value={a.color}
                  title={`Colour for ${a.label}`}
                  onChange={(e) => onColor(a.label, e.target.value)}
                  className="absolute inset-0 size-full cursor-pointer opacity-0"
                />
              )}
            </span>
            <span className="truncate text-ink">{a.label || "—"}</span>
            <span className="ml-auto shrink-0 font-mono text-ink-2">
              {Math.round(a.frac * 100)}%
            </span>
          </span>
        ))}
      </div>
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
  // A facility scope expands to its member airports at render (membership stays current).
  const dir = useFacilityDirectory();
  const icaos = widget.params?.facility
    ? facilityAirports(dir.data, widget.params.facility.id)
    : widget.params?.icaos?.length
      ? widget.params.icaos
      : widget.params?.icao
        ? [widget.params.icao]
        : [];
  const { rows, isLoading, isError, isFetching, dataUpdatedAt, refetch } = source.useRows({ icaos });
  useReportWidgetStatus(isFetching, dataUpdatedAt, refetch);
  const [ref, size] = useElementSize<HTMLDivElement>();
  const [configuring, setConfiguring] = useState(false);

  const isPie = widget.chartType === "pie";
  const aggregate = widget.aggregate ?? "none";
  const multiAirport = source.needsIcao && icaos.length > 1;
  const splitKey = multiAirport && widget.x !== AIRPORT_KEY ? AIRPORT_KEY : undefined;
  const normalize = !!widget.normalize;
  const thresholds = widget.thresholds ?? [];

  const shaped = useMemo(
    () =>
      shapeChartData(
        rows,
        source,
        widget.x,
        widget.y,
        aggregate,
        widget.chartType,
        widget.topN,
        splitKey,
        normalize,
      ),
    [rows, source, widget.x, widget.y, aggregate, widget.chartType, widget.topN, splitKey, normalize],
  );

  const colors = widget.colors ?? {};
  const chartTheme = useChartTheme();
  const colorFor = (key: string, i: number) => colors[key] ?? chartTheme.seriesAt(i);
  const setColor = (key: string, hex: string) =>
    onChange(widget.id, { colors: { ...colors, [key]: hex } });

  const categoryColors = widget.categoryColors ?? {};
  // The distinct x-categories in render order (same trim/order as what's actually drawn) — drives
  // both the per-datum bar/scatter recoloring below and the config panel's category swatch list.
  const categories = useMemo(
    () => (shaped.categorical ? shaped.data.map((r) => String(r[widget.x] ?? "")) : []),
    [shaped, widget.x],
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
        colorFor,
        categoryColors,
        normalize,
        thresholds,
        chartTheme.theme,
      ),
    // colorFor closes over `colors` + the theme; recompute when either (or thresholds) change.
    [shaped, widget.chartType, widget.x, xLabel, colors, categoryColors, normalize, thresholds, chartTheme],
  );
  // ChartPoint.markId === the series' `key` (set via `id:` in buildDefinition's marks) — this maps
  // a tooltip point back to its real, human series label instead of the mark's raw generated id.
  const labelByMarkId = useMemo(
    () => new Map(shaped.series.map((s) => [s.key, s.label])),
    [shaped.series],
  );

  const ready = widget.x && (aggregate === "count" || widget.y.length > 0);
  const empty = shaped.data.length === 0;
  const showChart = ready && !isError && !empty;

  return (
    <div className="flex h-full flex-col gap-2 p-2">
      {editing && (
        <div className="flex items-center justify-end">
          <Button size="sm" variant="secondary" className="h-7" onClick={() => setConfiguring(true)}>
            <Settings2 className="size-3.5" />
            Configure
          </Button>
        </div>
      )}
      <div ref={ref} className="min-h-0 flex-1">
        {isError ? (
          <EmptyState>Couldn&apos;t load data.</EmptyState>
        ) : !ready ? (
          <EmptyState>Open Configure and pick a group-by field.</EmptyState>
        ) : isLoading && empty ? (
          <EmptyState>Loading…</EmptyState>
        ) : empty ? (
          <EmptyState>No data.</EmptyState>
        ) : size.w <= 0 || size.h <= 0 ? null : isPie ? (
          <PieChart
            data={shaped.data}
            xKey={widget.x}
            valueKey={shaped.series[0]?.key ?? ""}
            size={size}
            colorFor={colorFor}
            editable={editing}
            onColor={setColor}
          />
        ) : (
          <Chart
            definition={definition}
            ariaLabel={source.label}
            width={size.w}
            height={size.h}
            renderTooltipBody={(ctx) =>
              ctx.points.length ? (
                <ChartTooltip
                  title={String(ctx.points[0].xValue)}
                  rows={ctx.points.map((p) => ({
                    color: p.color ?? "",
                    label: shaped.series.length > 1 ? labelByMarkId.get(p.markId) : undefined,
                    value: `${formatCompact(Number(p.yValue))}${normalize ? "%" : ""}`,
                  }))}
                />
              ) : null
            }
          />
        )}
      </div>
      {showChart && !isPie && shaped.series.length > 1 && (
        <Legend series={shaped.series} colorFor={colorFor} editable={editing} onColor={setColor} />
      )}
      {configuring && (
        <ChartConfigPanel
          source={source}
          widget={widget}
          series={shaped.series}
          categories={categories}
          multiAirport={multiAirport}
          onChange={onChange}
          onClose={() => setConfiguring(false)}
        />
      )}
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
    return <EmptyState>Unknown data source.</EmptyState>;
  }
  return (
    <ChartInner key={source.id} source={source} widget={widget} editing={editing} onChange={onChange} />
  );
}

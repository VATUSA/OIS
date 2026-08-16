import {useEffect, useMemo, useRef, useState} from "react";
import {Button} from "@ois/ui";
import {areaY, barY, type ChartValue, defineChart, lineY, ruleY} from "@tanstack/charts";
// The /tooltip entry is the same Chart with the built-in hover crosshair/focus enabled.
import {Chart} from "@tanstack/react-charts/tooltip";
import {scaleBand, scaleLinear, scalePoint} from "d3-scale";
import {Settings2} from "lucide-react";

import {ChartConfigPanel} from "./chart-config-panel";
import {
  AGGREGATES,
  type ChartType,
  colorAt,
  labelOf,
  type Series,
} from "./chart-shared";
import {AIRPORT_KEY, type DataSource, DATA_SOURCES_BY_ID, type Row} from "./sources";
import type {ChartAggregate, ChartThreshold, ChartWidget as ChartWidgetT} from "./types";

const COUNT_KEY = "__count";

const compactFmt = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
/** Compact axis numbers: 40000 → "40K", 1_500_000 → "1.5M", small values as-is. */
function fmtNumber(v: number): string {
  if (!Number.isFinite(v)) return "";
  return Math.abs(v) >= 1000 ? compactFmt.format(v) : String(Math.round(v * 100) / 100);
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
  normalized: boolean,
  thresholds: ChartThreshold[],
) {
  const x = xAccessor(xKey);
  const seriesMarks = series.map((s, i) => {
    const y = yAccessor(s.key);
    const color = colorFor(s.key, i);
    if (chartType === "line") return lineY(data, { x, y, stroke: color });
    if (chartType === "area") return areaY(data, { x, y, fill: color });
    return barY(data, { x, y, fill: color });
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
    : (v: ChartValue) => fmtNumber(Number(v));
  return defineChart({
    marks: [...seriesMarks, ...thresholdMarks],
    x: { scale: xScale, axis: { label: xLabel } },
    y: { scale: scaleLinear, axis: { label: yLabel, ticks: { format: yFormat } } },
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
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-1 text-xs text-muted-foreground">
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
  const icaos = widget.params?.icaos?.length
    ? widget.params.icaos
    : widget.params?.icao
      ? [widget.params.icao]
      : [];
  const { rows, isLoading, isError } = source.useRows({ icaos });
  const [ref, size] = useSize();
  const [configuring, setConfiguring] = useState(false);

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
  const colorFor = (key: string, i: number) => colors[key] ?? colorAt(i);
  const setColor = (key: string, hex: string) =>
    onChange(widget.id, { colors: { ...colors, [key]: hex } });

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
        normalize,
        thresholds,
      ),
    // colorFor closes over `colors`; recompute when colors/thresholds change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shaped, widget.chartType, widget.x, xLabel, colors, normalize, thresholds],
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
          <p className="pt-6 text-center text-sm text-muted-foreground">Couldn&apos;t load data.</p>
        ) : !ready ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">
            Open Configure and pick a group-by field.
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
            renderTooltipBody={(ctx) => {
              const pts = ctx.points;
              if (!pts.length) return null;
              return (
                <div className="pointer-events-none rounded-md border bg-popover px-2.5 py-1.5 text-xs shadow-lg">
                  <div className="mb-1 font-medium text-foreground">{String(pts[0].xValue)}</div>
                  <div className="flex flex-col gap-0.5">
                    {pts.map((p, i) => (
                      <div key={i} className="flex items-center gap-2">
                        <span
                          className="inline-block size-2 rounded-sm"
                          style={{ background: p.color }}
                        />
                        {p.groupLabel && (
                          <span className="text-muted-foreground">{p.groupLabel}</span>
                        )}
                        <span className="ml-auto tabular-nums text-foreground">
                          {fmtNumber(Number(p.yValue))}
                          {normalize ? "%" : ""}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              );
            }}
          />
        ) : null}
      </div>
      {showChart && shaped.series.length > 1 && (
        <Legend series={shaped.series} colorFor={colorFor} editable={editing} onColor={setColor} />
      )}
      {configuring && (
        <ChartConfigPanel
          source={source}
          widget={widget}
          series={shaped.series}
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
    return <div className="p-4 text-sm text-muted-foreground">Unknown data source.</div>;
  }
  return (
    <ChartInner key={source.id} source={source} widget={widget} editing={editing} onChange={onChange} />
  );
}

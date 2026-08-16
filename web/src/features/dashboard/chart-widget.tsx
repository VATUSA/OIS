import {useEffect, useMemo, useRef, useState} from "react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  usePrompt,
} from "@ois/ui";
import {areaY, barY, type ChartValue, defineChart, lineY} from "@tanstack/charts";
import {Chart} from "@tanstack/react-charts";
import {scaleBand, scaleLinear, scalePoint} from "d3-scale";
import {Check, Plus, X} from "lucide-react";

import {AIRPORT_KEY, type DataSource, DATA_SOURCES_BY_ID, type Row} from "./sources";
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
  // Prefer a low-cardinality categorical field for x over unique ids like callsign.
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

const labelOf = (source: DataSource, key: string) =>
  key === AIRPORT_KEY ? "Airport" : source.fields.find((f) => f.key === key)?.label ?? key;

interface Shaped {
  data: Row[];
  series: Series[];
  categorical: boolean;
}

/**
 * Shape rows for the chart:
 *  - "none": raw rows, series = the chosen y fields.
 *  - splitKey set (multi-airport, x is a real field): pivot — one series per airport, each x group
 *    aggregating the single y metric (or count). This is the "compare airports" mode.
 *  - otherwise: group by x, series = the y fields (or a single Count series).
 * Aggregated data keeps the top-N groups by value; bars stay value-ranked, lines/areas sort by x.
 */
function shapeChartData(
  rows: Row[],
  source: DataSource,
  xKey: string,
  yKeys: string[],
  aggregate: ChartAggregate,
  chartType: ChartType,
  topN: number | undefined,
  splitKey: string | undefined,
): Shaped {
  if (aggregate === "none") {
    const series = yKeys.map((k) => ({ key: k, label: labelOf(source, k) }));
    const xType = source.fields.find((f) => f.key === xKey)?.type;
    return { data: rows, series, categorical: xType !== "number" };
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
    return { data: orderXThenTrim(data, splitVals), series, categorical: true };
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
  return { data: orderXThenTrim(data, series.map((s) => s.key)), series, categorical: true };
}

function buildDefinition(
  data: Row[],
  chartType: ChartType,
  xKey: string,
  xLabel: string,
  categorical: boolean,
  series: Series[],
  colorFor: (key: string, i: number) => string,
) {
  const x = xAccessor(xKey);
  const marks = series.map((s, i) => {
    const y = yAccessor(s.key);
    const color = colorFor(s.key, i);
    if (chartType === "line") return lineY(data, { x, y, stroke: color });
    if (chartType === "area") return areaY(data, { x, y, fill: color });
    return barY(data, { x, y, fill: color });
  });
  const xScale = categorical ? (chartType === "bar" ? scaleBand : scalePoint) : scaleLinear;
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
      <DropdownMenuContent align="start" className="max-h-[50vh] w-48 overflow-y-auto">
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

function AirportsPicker({
  icaos,
  onChange,
}: {
  icaos: string[];
  onChange: (next: string[]) => void;
}) {
  const prompt = usePrompt();
  const add = async () => {
    const raw = await prompt({ title: "Add airport", label: "ICAO", placeholder: "KBOS" });
    if (!raw) return;
    const ic = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (ic.length >= 3 && !icaos.includes(ic)) onChange([...icaos, ic]);
  };
  return (
    <Picker label={`Airports · ${icaos.join(", ") || "none"}`}>
      <DropdownMenuLabel>Airports</DropdownMenuLabel>
      {icaos.map((ic) => (
        <DropdownMenuItem
          key={ic}
          onSelect={(e) => {
            e.preventDefault();
            if (icaos.length > 1) onChange(icaos.filter((x) => x !== ic));
          }}
        >
          <X className="size-3.5" />
          {ic}
        </DropdownMenuItem>
      ))}
      <DropdownMenuSeparator />
      <DropdownMenuItem onSelect={() => void add()}>
        <Plus className="size-3.5" />
        Add airport…
      </DropdownMenuItem>
    </Picker>
  );
}

function ConfigBar({
  source,
  widget,
  icaos,
  multiAirport,
  onChange,
}: {
  source: DataSource;
  widget: ChartWidgetT;
  icaos: string[];
  multiAirport: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const set = (patch: Record<string, unknown>) => onChange(widget.id, patch);
  const nums = source.fields.filter((f) => f.type === "number");
  const aggregate = widget.aggregate ?? "none";
  const aggLabel = AGGREGATES.find((a) => a.id === aggregate)?.label ?? "Count";
  const xLabel = labelOf(source, widget.x);
  const splitMode = multiAirport && widget.x !== AIRPORT_KEY;

  // Group-by options: real fields, plus "Airport" when comparing airports.
  const xOptions = [
    ...source.fields.map((f) => ({ key: f.key, label: f.label })),
    ...(multiAirport ? [{ key: AIRPORT_KEY, label: "Airport" }] : []),
  ];

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
      {source.needsIcao && (
        <AirportsPicker
          icaos={icaos}
          onChange={(next) => set({ params: { ...widget.params, icaos: next } })}
        />
      )}
      <Picker label={`Group by · ${xLabel}`}>
        <DropdownMenuLabel>X axis / group</DropdownMenuLabel>
        {xOptions.map((o) => (
          <CheckItem
            key={o.key}
            checked={widget.x === o.key}
            label={o.label}
            onSelect={() => set({ x: o.key })}
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
        <Picker label={splitMode ? `Metric · ${widget.y.length ? labelOf(source, widget.y[0]) : "—"}` : `Y · ${widget.y.length}`}>
          <DropdownMenuLabel>{splitMode ? "Metric (numeric)" : "Y series (numeric)"}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {nums.map((f) => (
            <CheckItem
              key={f.key}
              checked={ySet.has(f.key)}
              label={f.label}
              // In split (compare-airports) mode a single metric drives all airport series.
              onSelect={() => (splitMode ? set({ y: [f.key] }) : toggleY(f.key))}
              keepOpen={!splitMode}
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
            <span
              className="relative inline-block size-2.5 rounded-sm"
              style={{ background: color }}
            >
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

  const aggregate = widget.aggregate ?? "none";
  const multiAirport = source.needsIcao && icaos.length > 1;
  const splitKey = multiAirport && widget.x !== AIRPORT_KEY ? AIRPORT_KEY : undefined;

  const shaped = useMemo(
    () =>
      shapeChartData(rows, source, widget.x, widget.y, aggregate, widget.chartType, widget.topN, splitKey),
    [rows, source, widget.x, widget.y, aggregate, widget.chartType, widget.topN, splitKey],
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
      ),
    // colorFor closes over `colors`; recompute when colors change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shaped, widget.chartType, widget.x, xLabel, colors],
  );

  const ready = widget.x && (aggregate === "count" || widget.y.length > 0);
  const empty = shaped.data.length === 0;
  const showChart = ready && !isError && !empty;

  return (
    <div className="flex h-full flex-col gap-2 p-2">
      {editing && (
        <ConfigBar
          source={source}
          widget={widget}
          icaos={icaos}
          multiAirport={multiAirport}
          onChange={onChange}
        />
      )}
      <div ref={ref} className="min-h-0 flex-1">
        {isError ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">Couldn&apos;t load data.</p>
        ) : !ready ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">
            Pick a group-by field, and a metric unless counting.
          </p>
        ) : isLoading && empty ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">Loading…</p>
        ) : empty ? (
          <p className="pt-6 text-center text-sm text-muted-foreground">No data.</p>
        ) : size.w > 0 && size.h > 0 ? (
          <Chart definition={definition} ariaLabel={source.label} width={size.w} height={size.h} />
        ) : null}
      </div>
      {showChart && shaped.series.length > 1 && (
        <Legend series={shaped.series} colorFor={colorFor} editable={editing} onColor={setColor} />
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

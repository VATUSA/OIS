import * as React from "react";
import {barX, barY, defineChart, ruleX, ruleY, stack} from "@tanstack/charts";
import {tooltip} from "@tanstack/charts/tooltip";
import {scaleBand, scaleLinear} from "d3-scale";

import {ChartTooltip, formatCompact} from "./chart-tooltip";
import {ChartFrame} from "./frame";
import {tokenNames, useChartTheme, type ChartColor} from "./theme";

/**
 * Categorical bars, one value per category. `color` may vary per datum (a load level, a status);
 * `horizontal` lays categories down the y axis (long labels). `cap` draws a dashed warning rule.
 */
export function Bars<T>({
  data,
  category,
  value,
  color,
  horizontal = false,
  height = 220,
  label,
  valueFormat = formatCompact,
  cap,
}: {
  data: readonly T[];
  category: (d: T) => string;
  value: (d: T) => number;
  /** Token name or #hex per datum; defaults to `series-1`. */
  color?: (d: T) => ChartColor;
  horizontal?: boolean;
  height?: number | "fill";
  label: string;
  valueFormat?: (v: number) => string;
  cap?: number;
}) {
  const used = React.useMemo(() => tokenNames(color ? data.map(color) : []), [data, color]);
  const { theme, color: resolve, seriesAt, cap: capColor } = useChartTheme(used);
  const fill = (d: T) => (color ? resolve(color(d)) : seriesAt(0));

  const definition = React.useMemo(() => {
    const capMarks =
      cap == null
        ? []
        : horizontal
          ? [ruleX([cap], { x: (v: number) => v, stroke: capColor, strokeDasharray: "4 4", strokeWidth: 1 })]
          : [ruleY([cap], { y: (v: number) => v, stroke: capColor, strokeDasharray: "4 4", strokeWidth: 1 })];
    return horizontal
      ? defineChart({
          marks: [barX(data, { y: category, x: value, fill, inset: 2 }), ...capMarks],
          scales: {
            y: { scale: scaleBand },
            x: { scale: scaleLinear, grid: true, axis: { ticks: { format: (v) => valueFormat(Number(v)) } } },
          },
          theme,
          tooltip,
        })
      : defineChart({
          marks: [barY(data, { x: category, y: value, fill, inset: 2 }), ...capMarks],
          scales: {
            x: { scale: scaleBand },
            y: { scale: scaleLinear, grid: true, axis: { ticks: { format: (v) => valueFormat(Number(v)) } } },
          },
          theme,
          tooltip,
        });
    // colours derive from `theme`; the accessors are expected to be stable per data identity.
  }, [data, horizontal, cap, theme]);

  return (
    <ChartFrame
      definition={definition}
      label={label}
      height={height}
      renderTooltip={(ctx) => {
        const p = ctx.primaryPoint ?? ctx.points[0];
        if (!p) return null;
        const d = p.datum as T;
        return <ChartTooltip title={category(d)} rows={[{ color: fill(d), value: valueFormat(value(d)) }]} />;
      }}
    />
  );
}

export type StackKey = { key: string; label: string; color: ChartColor };
type StackRow = { category: string; key: string; value: number };

/**
 * Stacked bars: each category's `parts` stack in `keys` order. `cap` draws a dashed warning rule
 * (e.g. an AAR). Keys absent from a category count as zero.
 */
export function StackedBars({
  data,
  keys,
  height = 220,
  label,
  valueFormat = formatCompact,
  categoryFormat,
  cap,
}: {
  data: readonly { category: string; parts: Record<string, number> }[];
  keys: readonly StackKey[];
  height?: number | "fill";
  label: string;
  valueFormat?: (v: number) => string;
  categoryFormat?: (category: string) => string;
  cap?: number;
}) {
  const { theme, color, cap: capColor } = useChartTheme(tokenNames(keys.map((k) => k.color)));
  const colorOf = React.useMemo(() => new Map(keys.map((k) => [k.key, color(k.color)])), [keys, color]);
  const labelOf = React.useMemo(() => new Map(keys.map((k) => [k.key, k.label])), [keys]);

  const rows = React.useMemo<StackRow[]>(
    () => data.flatMap((d) => keys.map((k) => ({ category: d.category, key: k.key, value: d.parts[k.key] ?? 0 }))),
    [data, keys],
  );

  const definition = React.useMemo(
    () =>
      defineChart({
        marks: [
          barY(rows, {
            x: (r: StackRow) => r.category,
            y: (r: StackRow) => r.value,
            z: (r: StackRow) => r.key,
            fill: (r: StackRow) => colorOf.get(r.key) ?? theme.muted ?? "",
            layout: stack({ order: keys.map((k) => k.key) }),
            inset: 2,
          }),
          ...(cap == null ? [] : [ruleY([cap], { y: (v: number) => v, stroke: capColor, strokeDasharray: "4 4", strokeWidth: 1 })]),
        ],
        scales: {
          x: {
            scale: scaleBand,
            axis: categoryFormat ? { ticks: { format: (v) => categoryFormat(String(v)) } } : undefined,
          },
          y: { scale: scaleLinear, grid: true, axis: { ticks: { format: (v) => valueFormat(Number(v)) } } },
        },
        theme,
        tooltip,
      }),
    // colours derive from `theme`; the accessors are expected to be stable per data identity.
    [rows, colorOf, cap, theme],
  );

  return (
    <ChartFrame
      definition={definition}
      label={label}
      height={height}
      renderTooltip={(ctx) => {
        const first = ctx.points[0]?.datum as StackRow | undefined;
        if (!first) return null;
        const inCategory = rows.filter((r) => r.category === first.category && r.value > 0);
        const total = inCategory.reduce((a, r) => a + r.value, 0);
        return (
          <ChartTooltip
            title={`${categoryFormat ? categoryFormat(first.category) : first.category} · ${valueFormat(total)}`}
            rows={inCategory.map((r) => ({ color: colorOf.get(r.key) ?? "", label: labelOf.get(r.key), value: valueFormat(r.value) }))}
          />
        );
      }}
    />
  );
}

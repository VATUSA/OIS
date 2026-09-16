import * as React from "react";
import {areaY, defineChart, lineY, ruleY} from "@tanstack/charts";
import {tooltip} from "@tanstack/charts/tooltip";
import {scaleLinear, scaleTime} from "d3-scale";

import {ChartTooltip, formatCompact} from "./chart-tooltip";
import {ChartFrame} from "./frame";
import {tokenNames, useChartTheme, type ChartColor} from "./theme";

export type TimeSeriesSeries<T> = {
  key: string;
  label: string;
  value: (d: T) => number | null | undefined;
  /** Token name or #hex; defaults to the next `--series-*`. */
  color?: ChartColor;
};

export type Threshold = { value: number; label?: string };

/**
 * Line or area series over time (or any numeric x). Flat translucent area fills, faint grid, mono
 * tick labels via the theme, optional dashed warning thresholds.
 */
export function TimeSeries<T>({
  data,
  x,
  series,
  kind = "line",
  height = 220,
  label,
  yFormat = formatCompact,
  xFormat,
  thresholds = [],
}: {
  data: readonly T[];
  x: (d: T) => Date | number;
  series: readonly TimeSeriesSeries<T>[];
  kind?: "line" | "area";
  height?: number | "fill";
  label: string;
  yFormat?: (v: number) => string;
  xFormat?: (v: Date | number) => string;
  thresholds?: readonly Threshold[];
}) {
  const { theme, color, seriesAt, cap } = useChartTheme(tokenNames(series.map((s) => s.color)));
  const colors = series.map((s, i) => (s.color ? color(s.color) : seriesAt(i)));
  const byKey = new Map(series.map((s, i) => [s.key, { label: s.label, color: colors[i] }]));
  const isTime = data.length > 0 && x(data[0]) instanceof Date;

  const definition = React.useMemo(
    () =>
      defineChart({
        marks: [
          ...series.map((s, i) =>
            kind === "area"
              ? areaY(data, { x, y: (d: T) => s.value(d) ?? null, fill: colors[i], fillOpacity: 0.18, id: s.key })
              : lineY(data, { x, y: (d: T) => s.value(d) ?? null, stroke: colors[i], strokeWidth: 1.5, id: s.key }),
          ),
          ...(thresholds.length
            ? [ruleY(thresholds, { y: (t: Threshold) => t.value, stroke: cap, strokeDasharray: "4 4", strokeWidth: 1 })]
            : []),
        ],
        scales: {
          x: {
            scale: isTime ? scaleTime : scaleLinear,
            axis: xFormat ? { ticks: { format: (v) => xFormat(v as Date | number) } } : undefined,
          },
          y: { scale: scaleLinear, grid: true, axis: { ticks: { format: (v) => yFormat(Number(v)) } } },
        },
        theme,
        tooltip,
      }),
    // colours derive from `theme`; the accessors are expected to be stable per data identity.
    [data, series, kind, thresholds, theme, isTime],
  );

  return (
    <ChartFrame
      definition={definition}
      label={label}
      height={height}
      renderTooltip={(ctx) =>
        ctx.points.length ? (
          <ChartTooltip
            title={xFormat ? xFormat(ctx.points[0].xValue as Date | number) : String(ctx.points[0].xValue)}
            rows={ctx.points.map((p) => ({
              color: byKey.get(p.markId)?.color ?? p.color ?? theme.muted,
              label: series.length > 1 ? byKey.get(p.markId)?.label : undefined,
              value: yFormat(Number(p.yValue)),
            }))}
          />
        ) : null
      }
    />
  );
}

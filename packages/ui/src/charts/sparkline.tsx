import * as React from "react";
import {areaY, defineChart, dot, lineY} from "@tanstack/charts";
import {Chart} from "@tanstack/react-charts";
import {scaleLinear} from "d3-scale";

import type {Tone} from "../components/status-pill";
import {useChartTheme} from "./theme";

const TONE_TOKEN: Record<string, string> = {
  good: "success",
  warn: "warning",
  bad: "danger",
  brand: "brand",
  neutral: "ink-3",
};

type Point = { i: number; v: number };

/**
 * A metric-card sparkline (DESIGN.md): a flat translucent area, a 1.5px line and an emphasized
 * endpoint dot, all in one semantic hue, no guides.
 */
export function Sparkline({
  values,
  tone = "brand",
  width = 96,
  height = 38,
  label = "Trend",
}: {
  values: readonly number[];
  tone?: Tone;
  width?: number;
  height?: number;
  /** Accessible summary, e.g. "Audit events, last 30 days". */
  label?: string;
}) {
  const token = TONE_TOKEN[tone] ?? "brand";
  const { theme, color } = useChartTheme([token]);
  const hue = color(token);

  const definition = React.useMemo(() => {
    const data: Point[] = values.map((v, i) => ({ i, v }));
    const last = data.slice(-1);
    const x = (d: Point) => d.i;
    const y = (d: Point) => d.v;
    return defineChart({
      marks: [
        areaY(data, { x, y, fill: hue, fillOpacity: 0.16 }),
        lineY(data, { x, y, stroke: hue, strokeWidth: 1.5 }),
        dot(last, { x, y, r: 2, fill: hue, stroke: hue }),
      ],
      scales: { x: { scale: scaleLinear }, y: { scale: scaleLinear } },
      guides: false,
      margin: 3,
      theme,
    });
  }, [values, hue, theme]);

  if (values.length < 2) return null;
  return <Chart definition={definition} ariaLabel={label} width={width} height={height} />;
}

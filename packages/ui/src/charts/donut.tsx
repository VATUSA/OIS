import * as React from "react";

import {cn} from "../lib/utils";
import {useChartTheme, tokenNames, type ChartColor} from "./theme";

export type DonutSlice = { label: string; value: number; color: ChartColor };

/** SVG path for a ring segment from `a0`→`a1` (radians, 0 = 12 o'clock, clockwise). */
function segment(c: number, r0: number, r1: number, a0: number, a1: number): string {
  const p = (r: number, a: number) => `${c + r * Math.sin(a)} ${c - r * Math.cos(a)}`;
  const large = a1 - a0 > Math.PI ? 1 : 0;
  if (r0 <= 0) return `M${c} ${c} L${p(r1, a0)} A${r1} ${r1} 0 ${large} 1 ${p(r1, a1)} Z`;
  return `M${p(r1, a0)} A${r1} ${r1} 0 ${large} 1 ${p(r1, a1)} L${p(r0, a1)} A${r0} ${r0} 0 ${large} 0 ${p(r0, a0)} Z`;
}

/**
 * A donut (or pie, `thickness={1}`) of shares. The one chart drawn directly in SVG: the charting
 * engine's polar marks aren't ergonomic for a plain share ring. Flat fills, a `--card` hairline
 * between slices, colours from tokens; `center` renders inside the hole.
 */
export function Donut({
  slices,
  size = 120,
  thickness = 0.28,
  center,
  className,
  label,
}: {
  slices: readonly DonutSlice[];
  size?: number;
  /** Ring thickness as a fraction of the radius (1 = full pie). */
  thickness?: number;
  center?: React.ReactNode;
  className?: string;
  label?: string;
}) {
  const { theme, color } = useChartTheme(tokenNames(slices.map((s) => s.color)));
  const visible = slices.filter((s) => s.value > 0);
  const total = visible.reduce((a, s) => a + s.value, 0);
  const c = size / 2;
  const r1 = c - 1;
  const r0 = r1 * (1 - Math.min(1, Math.max(0, thickness)));

  let a = 0;
  return (
    <div className={cn("relative shrink-0", className)} style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={label}>
        {total <= 0 ? (
          <circle cx={c} cy={c} r={(r0 + r1) / 2} fill="none" stroke={theme.grid} strokeWidth={r1 - r0 || r1} />
        ) : visible.length === 1 ? (
          <circle cx={c} cy={c} r={(r0 + r1) / 2} fill="none" stroke={color(visible[0].color)} strokeWidth={r1 - r0 || r1} />
        ) : (
          visible.map((s) => {
            const a0 = a;
            a += (s.value / total) * Math.PI * 2;
            return (
              <path key={s.label} d={segment(c, r0, r1, a0, a)} fill={color(s.color)} stroke={theme.background} strokeWidth={1}>
                <title>{`${s.label}: ${s.value}`}</title>
              </path>
            );
          })
        )}
      </svg>
      {center != null && <div className="absolute inset-0 flex items-center justify-center text-center">{center}</div>}
    </div>
  );
}

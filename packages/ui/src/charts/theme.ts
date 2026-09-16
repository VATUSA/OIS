import * as React from "react";

import {useTokens} from "../lib/tokens";

const SERIES = ["series-1", "series-2", "series-3", "series-4", "series-5", "series-6", "series-7", "series-8"] as const;
const BASE = ["ink", "ink-3", "line-soft", "card", "warning"] as const;

/**
 * A chart colour: a design-token name without the leading `--` (`"series-3"`, `"flight-airborne"`,
 * `"success"`) or, for user-picked colours stored as data, a literal `#hex`.
 */
export type ChartColor = string;

/** Resolved chart theme + a colour resolver, re-read when the theme flips. */
export function useChartTheme(extra: readonly string[] = []) {
  const key = extra.join("|");
  // `key` stands in for `extra` so a fresh array literal each render doesn't re-resolve.
  const names = React.useMemo(() => [...BASE, ...SERIES, ...extra], [key]);
  const t = useTokens(names);
  return React.useMemo(() => {
    const color = (c: ChartColor): string => (c.startsWith("#") || c.startsWith("rgb") ? c : (t[c] ?? c));
    const series = SERIES.map((n) => t[n]);
    return {
      /** For `defineChart({ theme })`. */
      theme: { foreground: t.ink, muted: t["ink-3"], grid: t["line-soft"], background: t.card, palette: series },
      color,
      /** The i-th categorical series colour (cycles). */
      seriesAt: (i: number) => series[i % series.length],
      cap: t.warning,
    };
  }, [t]);
}

/** Every token name a set of chart colours references, for `useChartTheme(extra)`. */
export function tokenNames(colors: readonly (ChartColor | undefined)[]): string[] {
  return [...new Set(colors.filter((c): c is string => !!c && !c.startsWith("#") && !c.startsWith("rgb")))];
}

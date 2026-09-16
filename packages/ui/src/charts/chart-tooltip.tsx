import * as React from "react";

export type TooltipRow = { color: string; label?: React.ReactNode; value: React.ReactNode };

/** The one chart tooltip body: a title and colour-keyed rows. */
export function ChartTooltip({ title, rows }: { title: React.ReactNode; rows: readonly TooltipRow[] }) {
  return (
    <div className="pointer-events-none rounded-xs border border-line bg-panel-2 px-2 py-1.5 text-xs">
      <div className="mb-0.5 font-semibold text-ink">{title}</div>
      <div className="flex flex-col gap-0.5">
        {rows.map((r, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <span className="inline-block size-2 rounded-[2px]" style={{ background: r.color }} />
            {r.label != null && <span className="text-ink-2">{r.label}</span>}
            <span className="ml-auto pl-3 font-mono text-ink">{r.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

/** Compact numbers for axes/tooltips: 40000 → "40K", small values rounded to 2dp. */
const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
export function formatCompact(v: number): string {
  if (!Number.isFinite(v)) return "";
  return Math.abs(v) >= 1000 ? compact.format(v) : String(Math.round(v * 100) / 100);
}

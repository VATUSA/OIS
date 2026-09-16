import * as React from "react";
import {Maximize2, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {toneText, type Tone} from "./status-pill";

export type Trend = {
  direction: "up" | "down" | "flat";
  /** e.g. "23.5% (+10)". */
  text: React.ReactNode;
  /** Defaults: up → good, down → bad, flat → warn. Invert for "lower is better" metrics. */
  tone?: Tone;
};

const ARROW = { up: "▲", down: "▼", flat: "▶" } as const;
const TREND_TONE: Record<Trend["direction"], Tone> = { up: "good", down: "bad", flat: "warn" };

/**
 * A metric card (DESIGN.md): label, a big 700 tabular number, an optional semantic trend row and a
 * sparkline slot. The expand affordance appears when `onExpand` is set.
 */
export function MetricCard({
  label,
  value,
  sub,
  icon: Icon,
  tone,
  trend,
  sparkline,
  onExpand,
  className,
}: {
  label: React.ReactNode;
  value: React.ReactNode;
  sub?: React.ReactNode;
  icon?: LucideIcon;
  /** Colours the number itself (e.g. a live/offline count). */
  tone?: Tone;
  trend?: Trend;
  /** A `<Sparkline>` (or any small chart) pinned bottom-right. */
  sparkline?: React.ReactNode;
  onExpand?: () => void;
  className?: string;
}) {
  return (
    <div className={cn("relative overflow-hidden rounded-md border border-line bg-card p-4", className)}>
      <div className="flex items-center justify-between gap-2 text-xs text-ink-2">
        <span className="flex min-w-0 items-center gap-1.5 truncate">
          {Icon && <Icon className="size-3.5 shrink-0 text-ink-3" />}
          {label}
        </span>
        {onExpand && (
          <button type="button" onClick={onExpand} aria-label="Expand" className="text-ink-3 hover:text-ink">
            <Maximize2 className="size-3.5" />
          </button>
        )}
      </div>
      <div className={cn("mt-2 font-mono text-[26px] font-bold leading-none tracking-tight", tone ? toneText[tone] : "text-ink")}>
        {value}
      </div>
      {trend && (
        <div className={cn("mt-2 flex items-center gap-1 font-mono text-[11px]", toneText[trend.tone ?? TREND_TONE[trend.direction]])}>
          <span aria-hidden="true">{ARROW[trend.direction]}</span>
          {trend.text}
        </div>
      )}
      {sub && <div className="mt-1.5 text-xs text-ink-3">{sub}</div>}
      {sparkline && <div className="pointer-events-none absolute bottom-3 right-3 h-10 w-24">{sparkline}</div>}
    </div>
  );
}

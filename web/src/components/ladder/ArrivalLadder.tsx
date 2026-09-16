import {type ReactNode, useEffect, useRef, useState} from "react";
import {Button, DropdownMenu, DropdownMenuContent, DropdownMenuTrigger} from "@ois/ui";

import {hhmmZulu} from "@/lib/time";

import {bucketByTime, fitHorizontal, placeSlots} from "./layout";

/** Measures an element's rendered content width — the actual space available before the browser
 * would need to scroll it, which is what a same-time bucket must fit within (not the possibly-wider
 * auto-fit content box inside it). Mirrors `chart-widget.tsx`'s `useSize`. */
function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setWidth((prev) => (prev === el.clientWidth ? prev : el.clientWidth));
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

export interface ArrivalLadderItem<T> {
  key: string;
  /** Minutes from now (can be negative just after crossing/arrival). */
  min: number;
  /** Exact crossing/arrival instant — items sharing this exactly bucket into one slot. */
  time: string;
  data: T;
}

export interface ArrivalLadderProps<T> {
  items: ArrivalLadderItem<T>[];
  now: number;
  /** Window length, minutes. */
  win: number;
  pxPerMin: number;
  /** Left column width for the time axis. */
  gutter: number;
  /** Gridline interval, minutes. */
  step: number;
  /** Normal vertical spacing between slots. */
  rowGap: number;
  /** Floor the spacing can shrink to under density before slots start to overlap. */
  minGap: number;
  /** Top safety margin so the topmost slot never touches the container edge. */
  pad: number;
  renderTag: (data: T) => ReactNode;
  /** A CSS colour for the tick into each tag — pass a token, e.g. `var(--flight-airborne)`. */
  connectorColor: (data: T) => string;
  /** Estimated rendered pixel width of one tag, for auto-sizing and bucket overflow decisions. */
  measureTagWidth: (data: T) => number;
  /** Grow the content box to fit the widest tag (airport ladder) instead of a fixed `minWidth`
   * (FCA metering ladder). */
  autoFitWidth?: boolean;
  minWidth?: number;
  emptyMessage: string;
}

const GAP_PX = 6; // horizontal gap between tags sharing a bucket
const RIGHT_MARGIN = 12;

/**
 * Shared arrival/crossing ladder: plots items by time (NOW pinned at the bottom), decluttering
 * with a fixed axis and an adaptive gap that shrinks under density instead of stretching the
 * timeline. Items sharing the exact same `time` share one slot — laid out horizontally within the
 * ladder's actual rendered width, collapsing whatever doesn't fit into an expandable "+N" cluster
 * rather than ever forcing horizontal scroll or clipping.
 */
export function ArrivalLadder<T>({
  items,
  now,
  win,
  pxPerMin,
  gutter,
  step,
  rowGap,
  minGap,
  pad,
  renderTag,
  connectorColor,
  measureTagWidth,
  autoFitWidth,
  minWidth,
  emptyMessage,
}: ArrivalLadderProps<T>) {
  const [wrapRef, wrapWidth] = useWidth();

  const H = win * pxPerMin;
  const yOf = (min: number) => H - (Math.max(0, Math.min(min, win)) / win) * H;

  const filtered = items.filter((i) => i.min >= -1 && i.min <= win).sort((a, b) => a.min - b.min);
  const buckets = bucketByTime(filtered);
  const placed = placeSlots(buckets, { yOf, rowGap, minGap, height: H, pad });
  const contentH = H + pad;

  // Auto-fit sizes to the widest single tag, independent of buckets — a same-time cluster must
  // never widen the ladder itself; it only ever gets whatever width already exists, collapsing to
  // "+N" when that isn't enough (see the wrapWidth-driven fit below).
  const contentMinWidth = autoFitWidth
    ? Math.max(gutter + Math.ceil(filtered.reduce((m, i) => Math.max(m, measureTagWidth(i.data)), 0)), gutter + 96)
    : (minWidth ?? gutter + 96);

  // Unmeasured yet (first paint): assume unlimited width rather than prematurely collapsing —
  // the very next ResizeObserver tick corrects this before it's visually noticeable.
  const available = wrapWidth > 0 ? wrapWidth - gutter - RIGHT_MARGIN : Infinity;

  const grid = [];
  for (let k = 0; k <= win / step; k++) {
    const y = yOf(k * step);
    grid.push(
      <div key={k}>
        <div className="absolute border-t border-line-soft" style={{ top: y, left: gutter, right: 0 }} />
        <span
          className="absolute font-mono text-[10px] text-ink-3"
          style={{ top: y - 6, left: 0, width: gutter - 8, textAlign: "right" }}
        >
          {hhmmZulu(new Date(now + k * step * 60000).toISOString())}
        </span>
      </div>,
    );
  }

  if (placed.length === 0) {
    return <p className="py-6 text-center text-xs text-ink-3">{emptyMessage}</p>;
  }

  return (
    <div ref={wrapRef} className={autoFitWidth ? "overflow-x-auto pt-2" : "pt-2"}>
      <div className="relative" style={{ height: contentH, minWidth: contentMinWidth }}>
        <div className="absolute top-0 bottom-0 border-l border-line" style={{ left: gutter }} />
        {grid}
        <div className="absolute border-t border-brand" style={{ top: yOf(0), left: gutter, right: 0 }}>
          <span
            className="absolute -top-2 text-[10px] font-semibold text-brand-ink"
            style={{ left: 0, width: gutter - 8, textAlign: "right" }}
          >
            NOW
          </span>
        </div>

        {placed.map(({ y, bucket }) => {
          const { shown, overflowCount } =
            bucket.length > 1
              ? fitHorizontal(bucket, (i) => measureTagWidth(i.data), GAP_PX, available)
              : { shown: bucket, overflowCount: 0 };
          return (
            <div
              key={bucket[0].key}
              className="absolute flex items-center gap-1.5"
              style={{ top: y - 10, left: gutter }}
            >
              {shown.map((i) => (
                <span key={i.key} className="flex items-center">
                  <span className="h-0.5 w-3 shrink-0" style={{ background: connectorColor(i.data) }} />
                  {renderTag(i.data)}
                </span>
              ))}
              {overflowCount > 0 && (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-5 shrink-0 px-2 font-mono text-[10px]"
                    >
                      +{overflowCount}
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start" className="flex w-auto min-w-0 flex-col gap-1 p-1.5">
                    {bucket
                      .filter((i) => !shown.includes(i))
                      .map((i) => (
                        <span key={i.key} className="flex items-center">
                          <span
                            className="h-0.5 w-3 shrink-0"
                            style={{ background: connectorColor(i.data) }}
                          />
                          {renderTag(i.data)}
                        </span>
                      ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

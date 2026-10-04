import {Fragment} from "react";
import {Lock} from "lucide-react";
import {Button, DropdownMenu, DropdownMenuContent, DropdownMenuTrigger} from "@ois/ui";

import {bucketByTime, fitHorizontal, placeSlots} from "./layout";
import {type DelayLevel, delayLabel, tickMarks, type TguiColumn, type TguiItem} from "./tgui";

/**
 * The TGUI arrival ladder (VATUSA/OIS#557): one column per stream, each a vertical time axis with
 * NOW at the bottom. Every flight appears twice — its ETA on the left rail, and the time the flow
 * will deliver it (what the classic ladder plots) on the right, with the delay between them
 * printed beside it.
 *
 * Each rail is decluttered with the classic ladder's own geometry (`layout.ts`), unchanged: the axis
 * never stretches and NOW stays pinned (#62/#64), a same-time bucket claims one slot (#81), and tags
 * are only ever pushed *up* — a packed tag sits above its true time, and its leader line still points
 * at the true time on the rail. Tag areas have a fixed width, so a same-time cluster collapses to
 * "+N" inside it and can never widen its column.
 *
 * A view only: no rescheduling, swapping or dragging (that lives in #434).
 */

const TAG_AREA = 132; // px either side of the rails
const RAIL_GAP = 40; // px between the two rails — room for "HHMM"
const LEADER = 10; // px of horizontal leader between a tag and its rail
const TICK_MINOR = 4;
const TICK_MAJOR = 9;
const ROW_GAP = 18;
const MIN_GAP = 10;
const PAD = 8;
const CH = 7; // ≈ px per monospace char at text-[11px]
const CLUSTER_GAP = 4;
const CAPTION_H = 40;

const COLUMN_W = TAG_AREA * 2 + RAIL_GAP;
const LEFT_RAIL = TAG_AREA;
const RIGHT_RAIL = TAG_AREA + RAIL_GAP;

const LEVEL_CLASS: Record<DelayLevel, string> = {
  ok: "text-level-ok",
  watch: "text-level-watch",
  over: "text-level-over",
  early: "text-ink-3",
};

type Rail = "eta" | "sta";

interface RailItem {
  min: number;
  time: string;
  item: TguiItem;
}

function tagWidth(item: TguiItem, rail: Rail): number {
  const label = rail === "sta" ? (delayLabel(item.delayMin)?.text.length ?? 0) + 1 : 0;
  return 14 + (item.callsign.length + label) * CH + (item.committed ? 12 : 0);
}

function Tag({ item, rail }: { item: TguiItem; rail: Rail }) {
  const delay = rail === "sta" ? delayLabel(item.delayMin) : null;
  return (
    <span
      data-tag={rail}
      data-key={item.key}
      data-committed={item.committed || undefined}
      className={`flex shrink-0 items-center gap-1 rounded-xs border px-1 font-mono text-[11px] leading-4 ${
        item.committed ? "border-ink-3 bg-panel-2" : "border-transparent"
      }`}
    >
      {item.committed && <Lock className="size-2.5 text-ink-3" aria-label="committed" />}
      <span className="font-semibold">{item.callsign}</span>
      {delay && (
        <span data-delay={delay.level} className={LEVEL_CLASS[delay.level]}>
          {delay.text}
        </span>
      )}
    </span>
  );
}

function RailTags({
  items,
  rail,
  yOf,
  height,
}: {
  items: RailItem[];
  rail: Rail;
  yOf: (min: number) => number;
  height: number;
}) {
  const sorted = [...items].sort((a, b) => a.min - b.min);
  const placed = placeSlots(bucketByTime(sorted), { yOf, rowGap: ROW_GAP, minGap: MIN_GAP, height, pad: PAD });
  const left = rail === "eta";
  const railX = left ? LEFT_RAIL : RIGHT_RAIL;

  return (
    <>
      <svg className="pointer-events-none absolute inset-0" width={COLUMN_W} height={height} aria-hidden>
        {placed.map(({ y, bucket }) => {
          // The leader always ends at the true time on the rail, however far the tag was pushed up.
          const trueY = yOf(bucket[0].min);
          const tagX = left ? railX - TICK_MAJOR - LEADER : railX + TICK_MAJOR + LEADER;
          return (
            <line
              key={bucket[0].item.key}
              data-leader={rail}
              data-key={bucket[0].item.key}
              x1={tagX}
              y1={y}
              x2={railX}
              y2={trueY}
              style={{ stroke: "var(--ink-3)" }}
              strokeWidth={1}
            />
          );
        })}
      </svg>
      {placed.map(({ y, bucket }) => {
        const { shown, overflowCount } = fitHorizontal(
          bucket,
          (r) => tagWidth(r.item, rail),
          CLUSTER_GAP,
          TAG_AREA - TICK_MAJOR - LEADER - 28 /* room for "+N" */,
        );
        const hidden = bucket.filter((r) => !shown.includes(r));
        return (
          <div
            key={bucket[0].item.key}
            data-slot={rail}
            data-y={y}
            data-true-y={yOf(bucket[0].min)}
            className={`absolute flex items-center ${left ? "flex-row-reverse" : ""}`}
            style={{
              top: y - 8,
              gap: CLUSTER_GAP,
              ...(left
                ? { right: COLUMN_W - (railX - TICK_MAJOR - LEADER) }
                : { left: railX + TICK_MAJOR + LEADER }),
            }}
          >
            {shown.map((r) => (
              <Fragment key={r.item.key}>
                {left && r.item.wake && <span className="font-mono text-[10px] text-ink-3">{r.item.wake}</span>}
                <Tag item={r.item} rail={rail} />
              </Fragment>
            ))}
            {overflowCount > 0 && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button size="sm" variant="outline" className="h-4 shrink-0 px-1 font-mono text-[10px]">
                    +{overflowCount}
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="flex w-auto min-w-0 flex-col gap-1 p-1.5">
                  {hidden.map((r) => (
                    <Tag key={r.item.key} item={r.item} rail={rail} />
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
        );
      })}
    </>
  );
}

function Column({
  column,
  index,
  now,
  win,
  pxPerMin,
}: {
  column: TguiColumn;
  index: number;
  now: number;
  win: number;
  pxPerMin: number;
}) {
  const H = win * pxPerMin;
  const yOf = (min: number) => H - (Math.max(0, Math.min(min, win)) / win) * H;
  const inWindow = (min: number) => min >= -1 && min <= win;

  const eta: RailItem[] = column.items
    .filter((i) => i.etaMin != null && i.etaTime != null && inWindow(i.etaMin))
    .map((i) => ({ min: i.etaMin as number, time: i.etaTime as string, item: i }));
  const sta: RailItem[] = column.items
    .filter((i) => inWindow(i.staMin))
    .map((i) => ({ min: i.staMin, time: i.staTime, item: i }));
  const ticks = tickMarks(now, win);

  return (
    <div data-column={column.id} className="shrink-0" style={{ width: COLUMN_W }}>
      <div className="relative" style={{ height: H }}>
        <svg className="pointer-events-none absolute inset-0" width={COLUMN_W} height={H} aria-hidden>
          <line x1={LEFT_RAIL} y1={0} x2={LEFT_RAIL} y2={H} style={{ stroke: "var(--line)" }} />
          <line x1={RIGHT_RAIL} y1={0} x2={RIGHT_RAIL} y2={H} style={{ stroke: "var(--line)" }} />
          <line x1={LEFT_RAIL} y1={H} x2={RIGHT_RAIL} y2={H} style={{ stroke: "var(--brand)" }} />
          {ticks.map((t) => {
            const y = yOf(t.min);
            const len = t.major ? TICK_MAJOR : TICK_MINOR;
            return (
              <Fragment key={t.min}>
                <line x1={LEFT_RAIL - len} y1={y} x2={LEFT_RAIL} y2={y} style={{ stroke: "var(--ink-3)" }} />
                <line x1={RIGHT_RAIL} y1={y} x2={RIGHT_RAIL + len} y2={y} style={{ stroke: "var(--ink-3)" }} />
              </Fragment>
            );
          })}
        </svg>
        {ticks
          .filter((t) => t.label)
          .map((t) => (
            <span
              key={t.min}
              className="absolute text-center font-mono text-[10px] text-ink-2"
              style={{ top: yOf(t.min) - 6, left: LEFT_RAIL, width: RAIL_GAP }}
            >
              {t.label}
            </span>
          ))}
        <RailTags items={eta} rail="eta" yOf={yOf} height={H} />
        <RailTags items={sta} rail="sta" yOf={yOf} height={H} />
      </div>
      <div className="flex flex-col items-center gap-0.5 pt-2" style={{ height: CAPTION_H }}>
        <div className="flex items-center gap-1.5 font-mono text-[11px] text-ink">
          <span>{column.name}</span>
          <span className="rounded-xs border border-line px-1 text-ink-2">{index + 1}</span>
          <span>{column.name}</span>
        </div>
        <span className="font-mono text-[10px] text-ink-3">{column.kind}</span>
      </div>
    </div>
  );
}

export interface TguiLadderProps {
  columns: TguiColumn[];
  now: number;
  /** Window length, minutes. */
  win: number;
  pxPerMin?: number;
  emptyMessage: string;
}

export function TguiLadder({ columns, now, win, pxPerMin = 6, emptyMessage }: TguiLadderProps) {
  if (columns.every((c) => c.items.length === 0)) {
    return <p className="py-6 text-center text-xs text-ink-3">{emptyMessage}</p>;
  }
  return (
    <div data-ladder="tgui" className="overflow-x-auto pt-2">
      <div className="flex gap-4">
        {columns.map((c, i) => (
          <Column key={c.id} column={c} index={i} now={now} win={win} pxPerMin={pxPerMin} />
        ))}
      </div>
    </div>
  );
}

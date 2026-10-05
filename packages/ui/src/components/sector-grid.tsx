import * as React from "react";

import {cn} from "../lib/utils";

/** A cell's load against its sector's limit — `--level-ok` / `--level-watch` / `--level-over`. */
export type LoadLevel = "ok" | "watch" | "over";

export interface SectorGridCell {
  /** Peak count of active + proposed flights in the bin. */
  combined: number;
  /** Peak count of airborne (active) flights alone. */
  active: number;
  /** Decided by the caller (the alert rule lives server-side, #722); the grid only draws it. */
  level: LoadLevel;
}

export interface SectorGridRow {
  id: string;
  name?: string;
  limit: number;
  /** One per column of `binStarts`, in the same order. */
  cells: SectorGridCell[];
}

/** Tint + solid bar per level. Tokens only (DESIGN.md "Tokens only"); the figure itself stays ink. */
const LEVEL: Record<LoadLevel, string> = {
  ok: "bg-level-ok/15 border-level-ok",
  watch: "bg-level-watch/20 border-level-watch",
  over: "bg-level-over/20 border-level-over",
};

/** `HHMM` of an epoch-ms instant, in Zulu. */
function hhmm(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getUTCHours()).padStart(2, "0")}${String(d.getUTCMinutes()).padStart(2, "0")}`;
}

/** The leading columns stay put while the time axis scrolls under them. */
const STICKY = "sticky z-10 bg-card";

/**
 * A sector-by-time matrix (#724): one row per sector, one column per 15-minute bin. Not a `DataTable`
 * — its columns are a rolling time axis of load-carrying cells, not entity attributes (DESIGN.md
 * § Components, "Sector grid"). Purely presentational: the counts and each cell's level come from the
 * caller.
 *
 * Pass `onLimitChange` only where the viewer may edit limits; without it the limit cell is plain text
 * with no affordance at all.
 */
export function SectorGrid({
  rows,
  binStarts,
  caption,
  onLimitChange,
}: {
  rows: SectorGridRow[];
  binStarts: number[];
  caption: string;
  onLimitChange?: (sectorId: string, limit: number) => void;
}) {
  const times = binStarts.map(hhmm);
  return (
    <div className="w-full overflow-x-auto rounded-md border border-line-soft">
      <table className="border-collapse font-mono text-xs tabular-nums">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr className="text-ink-3">
            <th scope="col" className={cn(STICKY, "left-0 w-28 min-w-28 px-2 py-1.5 text-left font-sans font-semibold")}>
              Sector
            </th>
            <th scope="col" className={cn(STICKY, "left-28 w-14 min-w-14 border-r border-line-soft px-2 py-1.5 text-right font-sans font-semibold")}>
              Limit
            </th>
            {times.map((t, i) => (
              <th key={binStarts[i]} scope="col" className="sr-only">
                {t}Z
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id} className="border-t border-line-soft">
              <th scope="row" className={cn(STICKY, "left-0 w-28 min-w-28 px-2 py-1 text-left font-semibold text-ink")}>
                {row.id}
                {row.name && <span className="ml-1.5 font-sans text-ink-3">{row.name}</span>}
              </th>
              <td className={cn(STICKY, "left-28 w-14 min-w-14 border-r border-line-soft px-1 py-1 text-right text-ink-2")}>
                {onLimitChange ? (
                  <LimitInput sectorId={row.id} limit={row.limit} onCommit={onLimitChange} />
                ) : (
                  <span className="px-1">{row.limit}</span>
                )}
              </td>
              {row.cells.map((cell, i) => {
                const detail = `${row.id} ${times[i]}Z · peak ${cell.combined} (airborne ${cell.active}) vs limit ${row.limit}`;
                return (
                  <td key={binStarts[i]} className="p-0.5">
                    <span
                      data-level={cell.level}
                      title={detail}
                      aria-label={detail}
                      className={cn(
                        "block min-w-9 rounded-xs border-b-2 px-1.5 py-0.5 text-center font-semibold text-ink",
                        LEVEL[cell.level],
                      )}
                    >
                      {cell.combined}
                    </span>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr className="border-t border-line-soft text-ink-3">
            <td className={cn(STICKY, "left-0 w-28 min-w-28")} />
            <td className={cn(STICKY, "left-28 w-14 min-w-14 border-r border-line-soft")} />
            {times.map((t, i) => (
              <td key={binStarts[i]} className="px-0.5 py-1 text-center">
                {t}
              </td>
            ))}
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

/** The editable limit: commits a positive whole number on Enter or blur, reverts on Escape. */
function LimitInput({
  sectorId,
  limit,
  onCommit,
}: {
  sectorId: string;
  limit: number;
  onCommit: (sectorId: string, limit: number) => void;
}) {
  const [draft, setDraft] = React.useState(String(limit));
  // Escape reverts and then blurs; the blur must not commit the draft it just discarded.
  const cancelled = React.useRef(false);
  React.useEffect(() => setDraft(String(limit)), [limit]);
  const commit = () => {
    if (cancelled.current) {
      cancelled.current = false;
      setDraft(String(limit));
      return;
    }
    const next = Number(draft);
    if (Number.isInteger(next) && next > 0 && next !== limit) onCommit(sectorId, next);
    else setDraft(String(limit));
  };
  return (
    <input
      type="number"
      min={1}
      step={1}
      inputMode="numeric"
      aria-label={`Limit for ${sectorId}`}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setDraft(String(limit));
          // Only a focused input blurs, and only that blur must not commit.
          if (document.activeElement === e.currentTarget) {
            cancelled.current = true;
            e.currentTarget.blur();
          }
        }
      }}
      className="w-12 rounded-xs border border-line bg-panel-2 px-1 py-0.5 text-right font-mono text-xs text-ink hover:border-ink-3 focus:border-brand focus:outline-none"
    />
  );
}

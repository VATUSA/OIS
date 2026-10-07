import {type ReactNode, useCallback, useId, useState} from "react";
import {useQueryClient} from "@tanstack/react-query";
import {EmptyState, QueryState, SectorGrid, Select, Switch, useToast} from "@ois/ui";
import {ChevronDown, ChevronRight, Hourglass, MapPinOff} from "lucide-react";

import {useSetSectorLimit} from "@/features/sector-limits/sector-limits";
import {hhmmZulu} from "@/lib/time";

import {type SectorDemand, type SectorDemandTable, sectorDemandKey, useSectorDemand} from "./sector-demand";
import {
  type DemandTableKind,
  type DemandView,
  BIN_H,
  RANGE_MAX_H,
  RANGE_MIN_H,
  SPAN_CHOICES_H,
  formatHours,
  loadOpen,
  loadView,
  saveOpen,
  saveView,
  visibleGrid,
} from "./view";

const LABEL: Record<DemandTableKind, string> = { enroute: "Enroute", tracon: "TRACON" };

/** What a table's empty filter says: "No ZDC sectors alerting in the next 2.00 h" (#725). */
const noneAlerting = (artcc: string, kind: DemandTableKind, spanH: number) =>
  `No ${artcc}${kind === "tracon" ? " TRACON" : ""} sectors alerting in the next ${formatHours(spanH)}`;

/** Limit editing for the selected facility's own tables. Neighbours never get one. */
type LimitEditing = { onLimitChange: (sectorId: string, limit: number) => void; resets: number };

/**
 * One of an ARTCC's two tables, with its own controls: the 2–6 h range it draws and the "only sectors
 * alerting in the next N hours" filter, whose span is independent of the range. Both are remembered in
 * this browser per ARTCC and table. Every bin was computed server-side, so the controls only slice.
 */
export function DemandTable({
  artcc,
  kind,
  table,
  binStartsMs,
  editing,
}: {
  artcc: string;
  kind: DemandTableKind;
  table: SectorDemandTable;
  binStartsMs: readonly number[];
  editing?: LimitEditing;
}) {
  const [view, setView] = useState<DemandView>(() => loadView(artcc, kind));
  const update = (patch: Partial<DemandView>) => {
    const next = { ...view, ...patch };
    setView(next);
    saveView(artcc, kind, next);
  };
  const label = LABEL[kind];
  const name = `${artcc} ${label}`;
  const grid = visibleGrid(table.rows, binStartsMs, view);

  let body: ReactNode;
  if (!table.has_sector_data) {
    body = (
      <EmptyState icon={MapPinOff} title={`No ${label} sector data for ${artcc}`} className="rounded-md border border-line-soft">
        The sector dataset has no {kind === "tracon" ? "Approach Control" : "Low, High or Ultra High"} volumes for{" "}
        {artcc}. That is a gap in the data, not a quiet sky.
      </EmptyState>
    );
  } else if (table.rows.length === 0) {
    body = <EmptyState className="rounded-md border border-line-soft">{`No ${name} sectors to show.`}</EmptyState>;
  } else if (grid.rows.length === 0) {
    body = (
      <EmptyState className="rounded-md border border-line-soft">{noneAlerting(artcc, kind, view.alertSpanH)}</EmptyState>
    );
  } else {
    body = (
      <SectorGrid
        // A refused write remounts the grid, so a limit input shows the stored value, not the typed one.
        key={editing?.resets ?? 0}
        rows={grid.rows}
        binStarts={grid.binStarts}
        caption={`${name} sector demand`}
        onLimitChange={editing?.onLimitChange}
      />
    );
  }

  return (
    <section aria-label={`${name} sectors`} className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
        <h2 className="text-sm font-semibold text-ink">{label}</h2>
        {table.has_sector_data && table.rows.length > 0 && (
          <>
            <label className="flex items-center gap-2 text-xs text-ink-2">
              Range
              <input
                type="range"
                aria-label={`${name} range`}
                min={RANGE_MIN_H}
                max={RANGE_MAX_H}
                step={BIN_H}
                value={view.rangeH}
                onChange={(e) => update({ rangeH: Number(e.target.value) })}
                className="w-28 accent-brand"
              />
              <span className="w-14 font-mono tabular-nums text-ink">{formatHours(view.rangeH)}</span>
            </label>
            <span className="flex items-center gap-2 text-xs text-ink-2">
              <Switch
                aria-label={`${name}: only sectors alerting`}
                checked={view.alertOnly}
                onCheckedChange={(alertOnly) => update({ alertOnly })}
              />
              Only alerting in the next
              <Select
                aria-label={`${name} alert span`}
                size="sm"
                value={String(view.alertSpanH)}
                onChange={(e) => update({ alertSpanH: Number(e.target.value) })}
                className="font-mono tabular-nums"
              >
                {SPAN_CHOICES_H.map((h) => (
                  <option key={h} value={String(h)}>
                    {formatHours(h)}
                  </option>
                ))}
              </Select>
            </span>
          </>
        )}
      </div>
      {body}
    </section>
  );
}

/**
 * Everything one demand body draws: the no-data and before-first-cycle states, each naming the
 * facility, or the enroute and TRACON tables. Neither state is ever an empty grid (#725).
 */
function DemandBody({ data, editing }: { data: SectorDemand; editing?: LimitEditing }) {
  const { artcc } = data;
  if (data.status === "no_sector_data") {
    return (
      <EmptyState icon={MapPinOff} title={`No sector data for ${artcc}`} className="rounded-md border border-line">
        The sector dataset has no volumes for {artcc}, so its demand can&apos;t be counted. That is a gap in the
        data, not a quiet sky.
      </EmptyState>
    );
  }
  if (data.status === "pending") {
    return (
      <EmptyState icon={Hourglass} title="Waiting for the first feed cycle" className="rounded-md border border-line">
        {artcc}&apos;s sector demand is counted from the live feed and appears once the server has received a cycle.
      </EmptyState>
    );
  }
  return (
    <div className="flex min-w-0 flex-col gap-6">
      <DemandTable artcc={artcc} kind="enroute" table={data.enroute} binStartsMs={data.bin_starts_ms} editing={editing} />
      <DemandTable artcc={artcc} kind="tracon" table={data.tracon} binStartsMs={data.bin_starts_ms} editing={editing} />
    </div>
  );
}

/** Writes the facility's own limits; the demand refetches so its cells recolour against the new one. */
function useLimitEditing(artcc: string): LimitEditing {
  const qc = useQueryClient();
  const toast = useToast();
  const { mutate } = useSetSectorLimit(artcc);
  const [resets, setResets] = useState(0);
  const onLimitChange = useCallback(
    (sectorId: string, limit: number) =>
      mutate(
        { sectorId, limit },
        {
          onSuccess: () => void qc.invalidateQueries({ queryKey: sectorDemandKey(artcc) }),
          onError: () => {
            setResets((n) => n + 1);
            toast.error(`Couldn't set ${sectorId}'s limit`, { description: "The limit is unchanged." });
          },
        },
      ),
    [mutate, qc, artcc, toast],
  );
  return { onLimitChange, resets };
}

/**
 * A neighbour's tables: collapsed by default, open state remembered per browser per facility, and
 * view-only whatever the viewer may edit there — a neighbour's limits are theirs to set. The server
 * refuses the write too (403 outside the viewer's scope); here the grid gets no editor at all. Nothing
 * is fetched until it is opened.
 */
function NeighbourPanel({ pageArtcc, artcc }: { pageArtcc: string; artcc: string }) {
  const [open, setOpen] = useState(() => loadOpen(pageArtcc, artcc));
  const q = useSectorDemand(artcc, { enabled: open });
  const panelId = useId();
  const toggle = () => {
    setOpen(!open);
    saveOpen(pageArtcc, artcc, !open);
  };
  return (
    <div className="min-w-0 rounded-md border border-line">
      <button
        type="button"
        onClick={toggle}
        aria-expanded={open}
        aria-controls={panelId}
        className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-left hover:bg-panel-2"
      >
        {open ? (
          <ChevronDown className="size-3.5 shrink-0 text-ink-3" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0 text-ink-3" />
        )}
        <span className="font-mono text-sm font-semibold text-ink">{artcc}</span>
        <span className="ml-auto text-xs text-ink-3">View only</span>
      </button>
      {open && (
        <div id={panelId} className="border-t border-line-soft p-3">
          <QueryState
            isLoading={q.isLoading}
            isError={q.isError}
            onRetry={() => void q.refetch()}
            loading={`Loading ${artcc}…`}
            error={`Couldn't load ${artcc}'s sector demand.`}
          >
            {q.data && <DemandBody data={q.data} />}
          </QueryState>
        </div>
      )}
    </div>
  );
}

/**
 * The selected facility's whole set (#725): its enroute table, its TRACON table, then its neighbours.
 * The page keys this by facility, so switching facility replaces all of it — no table, control or open
 * neighbour of the previous facility survives the switch.
 */
export function FacilityDemand({ artcc }: { artcc: string }) {
  const q = useSectorDemand(artcc);
  const editing = useLimitEditing(artcc);
  const data = q.data;
  return (
    <QueryState
      isLoading={q.isLoading}
      isError={q.isError && !data}
      onRetry={() => void q.refetch()}
      loading={`Loading ${artcc}…`}
      error={`Couldn't load ${artcc}'s sector demand.`}
      className="rounded-md border border-line"
    >
      {data && (
        <div className="flex min-w-0 flex-col gap-8">
          <div className="flex min-w-0 flex-col gap-3">
            {data.status === "ready" && (
              <p className="text-xs text-ink-3">
                Peak one-minute occupancy per Zulu quarter-hour, counted from the{" "}
                <span className="font-mono tabular-nums text-ink-2">{hhmmZulu(data.cycle_at)}</span> feed cycle.
              </p>
            )}
            <DemandBody data={data} editing={data.limits_editable ? editing : undefined} />
          </div>
          {data.neighbours.length > 0 && (
            <section aria-label={`${artcc} neighbours`} className="flex min-w-0 flex-col gap-2">
              <h2 className="text-sm font-semibold text-ink">Neighbours</h2>
              {data.neighbours.map((n) => (
                <NeighbourPanel key={n} pageArtcc={artcc} artcc={n} />
              ))}
            </section>
          )}
        </div>
      )}
    </QueryState>
  );
}

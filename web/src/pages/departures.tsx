import {useMemo, useState} from "react";
import {useIsMutating} from "@tanstack/react-query";
import {Button, type DataColumn, DataTable, Input, MetricCard, QueryState, StatusPill} from "@ois/ui";
import {Clock, Plane, PlaneTakeoff} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type Departure, useDepartures, useIssueCfr, useReleaseCfr} from "@/lib/departures";
import {useHistoricalAt} from "@/lib/historical-context";
import {hasPermission} from "@/lib/permissions";
import {hhmmZulu, parseHhmm} from "@/lib/time";

function delayClass(min: number): string {
  if (min >= 15) return "text-level-over";
  if (min > 0) return "text-level-watch";
  return "text-ink-3";
}

/** Whether a CFR issue/release for this callsign is in flight (the row's editors share it). */
function useRowBusy(callsign: string): boolean {
  return (
    useIsMutating({
      predicate: (m) => {
        const v = m.state.variables as { callsign?: string } | string | undefined;
        return (typeof v === "string" ? v : v?.callsign) === callsign;
      },
    }) > 0
  );
}

/** Ready-time editor: issue a CFR for a pilot-given HHMMz. */
function ReadyCell({ d, canIssue }: { d: Departure; canIssue: boolean }) {
  const issue = useIssueCfr();
  const busy = useRowBusy(d.callsign);
  const [ready, setReady] = useState("");

  if (!(d.has_program && !d.cfr_issued && canIssue)) return <span className="text-ink-3">—</span>;

  function setReadyTime() {
    const iso = parseHhmm(ready);
    if (!iso) return;
    issue.mutate({ callsign: d.callsign, airport: d.arrival, readyTime: iso }, { onSuccess: () => setReady("") });
  }

  return (
    <div className="flex items-center gap-1">
      <Input
        aria-label={`Ready time for ${d.callsign}`}
        className="h-8 w-20 font-mono text-xs"
        placeholder="HHMMz"
        value={ready}
        onChange={(e) => setReady(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && setReadyTime()}
      />
      <Button size="sm" variant="outline" disabled={busy || !parseHhmm(ready)} onClick={setReadyTime}>
        Set
      </Button>
    </div>
  );
}

function ActionsCell({ d, canIssue }: { d: Departure; canIssue: boolean }) {
  const issue = useIssueCfr();
  const release = useReleaseCfr();
  const busy = useRowBusy(d.callsign);

  if (!d.has_program) return <span className="text-xs text-ink-3">Release at will</span>;
  if (!canIssue) return null;
  return (
    <div className="flex justify-end gap-1">
      {d.cfr_issued ? (
        <Button
          size="sm"
          variant="ghost"
          className="text-danger hover:text-danger"
          disabled={busy}
          onClick={() => release.mutate(d.callsign)}
        >
          Cancel
        </Button>
      ) : (
        <>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => issue.mutate({ callsign: d.callsign, airport: d.arrival })}
          >
            Issue CFR
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() =>
              issue.mutate({ callsign: d.callsign, airport: d.arrival, readyTime: new Date().toISOString() })
            }
          >
            Release now
          </Button>
        </>
      )}
    </div>
  );
}

const timeKey = (iso: string | null | undefined) => (iso ? new Date(iso).getTime() : Infinity);

function departureColumns(canIssue: boolean): DataColumn<Departure>[] {
  return [
    { accessorKey: "dep", header: "From", mono: true },
    {
      accessorKey: "callsign",
      header: "Callsign",
      icon: Plane,
      mono: true,
      cell: (c) => <span className="font-semibold">{c.row.original.callsign}</span>,
    },
    { accessorKey: "arrival", header: "To", mono: true },
    { id: "aircraft_type", accessorFn: (d) => d.aircraft_type ?? "", header: "Type", mono: true },
    {
      id: "gate",
      accessorFn: (d) => d.gate ?? "",
      header: "Gate",
      mono: true,
      cell: (c) => c.row.original.gate ?? "—",
    },
    {
      id: "eta",
      accessorFn: (d) => timeKey(d.eta),
      header: "ETA",
      icon: Clock,
      mono: true,
      sortDescFirst: false,
      cell: (c) => <span className="text-ink-2">{hhmmZulu(c.row.original.eta)}</span>,
    },
    {
      accessorKey: "delay_min",
      header: "Delay",
      mono: true,
      align: "right",
      cell: (c) => {
        const d = c.row.original.delay_min;
        return <span className={delayClass(d)}>{d > 0 ? `+${d}` : "—"}</span>;
      },
    },
    {
      id: "cfr",
      accessorFn: (d) => timeKey(d.cfr),
      header: "CFR",
      icon: PlaneTakeoff,
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => {
        const d = c.row.original;
        if (!d.cfr) return <span className="text-ink-3">—</span>;
        const releaseNow = new Date(d.cfr).getTime() <= Date.now() + 60_000;
        return <span className={releaseNow ? "text-success" : "text-warning"}>{hhmmZulu(d.cfr)}</span>;
      },
    },
    {
      id: "ready",
      header: "Ready",
      enableSorting: false,
      cell: (c) => <ReadyCell d={c.row.original} canIssue={canIssue} />,
    },
    {
      id: "state",
      accessorFn: (d) => (!d.has_program ? "no program" : d.cfr_issued ? "issued" : "proposed"),
      header: "State",
      cell: (c) => {
        const d = c.row.original;
        return !d.has_program ? (
          <StatusPill tone="neutral">no program</StatusPill>
        ) : d.cfr_issued ? (
          <StatusPill tone="good">issued</StatusPill>
        ) : (
          <StatusPill tone="brand">proposed</StatusPill>
        );
      },
    },
    {
      id: "actions",
      header: "",
      enableSorting: false,
      align: "right",
      cell: (c) => <ActionsCell d={c.row.original} canIssue={canIssue} />,
    },
  ];
}

/**
 * Departures out of a single airport — a tab on the Airport page. The airport picker + header
 * live on that page, so this view just takes the loaded ICAO and renders the picture.
 */
export function DeparturesView({ icao }: { icao: string }) {
  const { data: me } = useMe();
  const departures = useDepartures(icao);
  // In historical replay the board is read-only — CFR actions would hit the live feed.
  const replay = useHistoricalAt() != null;
  const canIssue = hasPermission(me, "tmu.cfr.assign") && !replay;
  const columns = useMemo(() => departureColumns(canIssue), [canIssue]);
  const data = departures.data;

  return (
    <QueryState
      isLoading={!data && !departures.isError}
      isError={departures.isError}
      loading="Loading departures…"
      error={`Couldn't load departures for ${icao}.`}
      onRetry={() => departures.refetch()}
      className="rounded-md border border-line py-12"
    >
      {data && (
        <div className="flex flex-col gap-4">
          <div className="grid gap-3 sm:grid-cols-3">
            <MetricCard label="Total" icon={PlaneTakeoff} value={data.total} />
            <MetricCard label="To metered fields" value={data.to_metered} />
            <MetricCard label="Holding on CFR" value={data.holding_on_cfr} />
          </div>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm text-ink-2">
            {data.facility_kind && (
              <span>
                <span className="font-semibold capitalize text-ink">{data.facility_kind}</span> facility ·{" "}
                <span className="font-mono text-ink">{data.airports.join(" ")}</span>
              </span>
            )}
            <span>
              Destinations with a TMU program:{" "}
              {data.program_destinations.length ? (
                <span className="font-mono text-ink">{data.program_destinations.join(", ")}</span>
              ) : (
                "none"
              )}
            </span>
          </div>
          <DataTable
            label={`Departures out of ${icao}`}
            columns={columns}
            data={data.departures}
            getRowId={(d) => d.callsign}
            rowCap={25}
            empty={`No pending departures out of ${icao}.`}
          />
        </div>
      )}
    </QueryState>
  );
}

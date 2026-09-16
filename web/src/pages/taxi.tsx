import {useEffect, useMemo, useState} from "react";
import {type DataColumn, DataTable, MetricCard, StatusPill} from "@ois/ui";
import {Plane, Timer} from "lucide-react";

import {useHistoricalAt} from "@/lib/historical-context";
import {type TaxiActive, useTaxiStats} from "@/lib/taxi";

/** A clock that ticks every second, so rolling timers advance between polls. */
function useNow() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}

function fmtClock(secs: number) {
  const s = Math.max(0, Math.floor(secs));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

type Row = TaxiActive & { field: string };

function TaxiTimer({ row, now }: { row: Row; now: number }) {
  if (row.phase !== "rolling" || !row.rolling_since) return <span className="text-ink-3">—</span>;
  return <span className="text-warning">{fmtClock((now - new Date(row.rolling_since).getTime()) / 1000)}</span>;
}

function taxiColumns(now: number): DataColumn<Row>[] {
  return [
    { accessorKey: "field", header: "Field", mono: true, cellClassName: "font-semibold" },
    { accessorKey: "callsign", header: "Callsign", icon: Plane, mono: true, cellClassName: "font-semibold" },
    {
      id: "dest",
      accessorFn: (r) => r.dest || "",
      header: "Dest",
      mono: true,
      cell: (c) => <span className="text-ink-2">{c.row.original.dest || "—"}</span>,
    },
    { accessorKey: "gs", header: "GS", mono: true, align: "right" },
    { accessorKey: "alt", header: "Alt", mono: true, align: "right" },
    {
      accessorKey: "phase",
      header: "Phase",
      cell: (c) => (
        <StatusPill tone={c.row.original.phase === "rolling" ? "warn" : "brand"}>{c.row.original.phase}</StatusPill>
      ),
    },
    {
      id: "taxi",
      header: "Taxi time",
      icon: Timer,
      mono: true,
      align: "right",
      enableSorting: false,
      cell: (c) => <TaxiTimer row={c.row.original} now={now} />,
    },
  ];
}

/**
 * Live taxi-out picture for a single airport — a tab on the Airport page. From the start of the
 * roll ({">"}7 kt) to wheels-up (60 kt or a climb).
 */
export function TaxiView({ icao }: { icao: string }) {
  const liveNow = useNow();
  const at = useHistoricalAt();
  // In replay the rolling timer must tick against the scrubber instant, not the wall clock.
  const now = at != null ? at * 1000 : liveNow;
  const stats = useTaxiStats(icao);

  const rows: Row[] = useMemo(
    () => (stats.data?.active ?? []).map((a) => ({ ...a, field: icao })),
    [stats.data, icao],
  );
  const columns = useMemo(() => taxiColumns(now), [now]);
  const summary = {
    rolling: rows.filter((r) => r.phase === "rolling").length,
    watching: rows.filter((r) => r.phase === "watching").length,
    samples: stats.data?.sample_count ?? 0,
    avg: stats.data?.avg_min ?? null,
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <MetricCard label="Rolling" value={summary.rolling} tone={summary.rolling > 0 ? "warn" : undefined} />
        <MetricCard label="Watching" value={summary.watching} tone={summary.watching > 0 ? "brand" : undefined} />
        <MetricCard label="Avg today" value={summary.avg != null ? `${summary.avg}m` : "—"} />
        <MetricCard
          label="Samples"
          value={summary.samples}
          sub={at != null ? "replay" : "live · updates every 15s"}
        />
      </div>

      <DataTable
        label={`Taxiing out of ${icao}`}
        columns={columns}
        data={rows}
        getRowId={(r) => `${r.field}-${r.callsign}`}
        // Live ops list: always pages, never hides rows behind "Show all".
        rowCap={Infinity}
        pageSize={25}
        isLoading={stats.isLoading}
        isError={stats.isError}
        onRetry={() => stats.refetch()}
        empty={`No departures taxiing out of ${icao} right now. Rows appear as aircraft push and roll.`}
      />
    </div>
  );
}

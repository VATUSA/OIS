import {useState} from "react";
import {Link} from "@tanstack/react-router";
import {
  buttonVariants,
  Card,
  type DataColumn,
  DataTable,
  EmptyState,
  MetricCard,
  QueryState,
  StatusPill,
} from "@ois/ui";
import {Clock, OctagonX, Plane, Split, Timer, TrafficCone, Waypoints} from "lucide-react";

import {FlightSearch} from "@/components/flight-search";
import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {useTraffic} from "@/lib/fca";
import {toneOf} from "@/lib/status";
import {hhmmZulu} from "@/lib/time";
import {
  type FlightAdvisory,
  type PublicRestriction,
  useMyFlight,
  usePublicBoard,
  usePublicFlight,
} from "@/lib/public";

/** One initiative affecting the flight, flattened so every kind shares the table's columns. */
type Constraint = {
  id: string;
  icon: typeof Timer;
  kind: string;
  where: string;
  /** An FCA's own saved colour (user data). */
  color?: string;
  detail: React.ReactNode;
  time: string;
  /** Minutes of delay, or null when the initiative assigns none. */
  delay: number | null;
};

function constraintsOf(f: FlightAdvisory): Constraint[] {
  const out: Constraint[] = [];
  if (f.gdp) {
    out.push({
      id: "gdp",
      icon: Timer,
      kind: "Ground Delay Program",
      where: f.gdp.airport,
      detail: (
        <>
          AAR <span className="font-mono">{f.gdp.aar}</span> ·{" "}
          {f.gdp.controlled ? (
            <span className="font-mono">EDCT {hhmmZulu(f.gdp.edct)}</span>
          ) : (
            "subject to the program (no slot assigned yet)"
          )}
        </>
      ),
      time: `${f.gdp.start_time}z–${f.gdp.end_time}z`,
      delay: f.gdp.controlled ? f.gdp.delay_min : null,
    });
  }
  if (f.ground_stop) {
    out.push({
      id: "gs",
      icon: OctagonX,
      kind: "Ground Stop",
      where: f.ground_stop.airport,
      detail: <span className="font-mono">{f.ground_stop.scope.trim() === "" ? "field-wide" : f.ground_stop.scope}</span>,
      time: f.ground_stop.until ? `until ${f.ground_stop.until}z` : "until further notice",
      delay: null,
    });
  }
  if (f.rate_program) {
    out.push({
      id: "metering",
      icon: Split,
      kind: "Arrival metering",
      where: f.rate_program.airport,
      detail: (
        <>
          AAR <span className="font-mono">{f.rate_program.aar}</span>
        </>
      ),
      time: f.rate_program.sta ? `STA ${hhmmZulu(f.rate_program.sta)}` : "—",
      delay: f.rate_program.delay_min,
    });
  }
  for (const x of f.fcas) {
    out.push({
      id: `fca-${x.fca_id}`,
      icon: Waypoints,
      kind: "Flow Constrained Area",
      where: x.fca_name,
      color: x.color,
      detail: x.edct ? <span className="font-mono">EDCT {hhmmZulu(x.edct)}</span> : "—",
      time: x.cross_time ? `cross ${hhmmZulu(x.cross_time)}` : "—",
      delay: x.delay_min,
    });
  }
  return out;
}

const constraintColumns: DataColumn<Constraint>[] = [
  {
    accessorKey: "kind",
    header: "Initiative",
    cell: (c) => {
      const Icon = c.row.original.icon;
      return (
        <span className="flex items-center gap-2 font-semibold text-ink">
          <Icon className="size-4 text-ink-3" />
          {c.row.original.kind}
        </span>
      );
    },
  },
  {
    accessorKey: "where",
    header: "Where",
    mono: true,
    cell: (c) => (
      <span className="flex items-center gap-2">
        {c.row.original.color && (
          <span className="size-2.5 shrink-0 rounded-full" style={{ background: c.row.original.color }} />
        )}
        {c.row.original.where}
      </span>
    ),
  },
  { id: "detail", header: "Detail", enableSorting: false, cell: (c) => <span className="text-ink-2">{c.row.original.detail}</span> },
  { accessorKey: "time", header: "Time", mono: true },
  {
    id: "delay",
    accessorFn: (r) => r.delay ?? -1,
    header: "Delay",
    align: "right",
    cell: (c) => {
      const d = c.row.original.delay;
      if (d == null) return <span className="text-ink-3">—</span>;
      return d > 0 ? <StatusPill tone="bad">{d} min delay</StatusPill> : <StatusPill tone="neutral">no delay</StatusPill>;
    },
  },
];

const restrictionColumns: DataColumn<PublicRestriction>[] = [
  {
    id: "restriction",
    accessorFn: (r) => r.decoded || r.restriction,
    header: "Restriction",
  },
  {
    id: "facilities",
    accessorFn: (r) => `${r.requesting} → ${r.providing}`,
    header: "Req → Prov",
    mono: true,
  },
  {
    id: "window",
    accessorFn: (r) => r.start_time,
    header: "Window",
    mono: true,
    align: "right",
    cell: (c) => {
      const r = c.row.original;
      return `${hhmmZulu(r.start_time)}${r.stop_time ? `–${hhmmZulu(r.stop_time)}` : " · UFN"}`;
    },
  },
];

function Result({ f }: { f: FlightAdvisory }) {
  if (!f.found) {
    return (
      <EmptyState icon={Plane} title={<span className="font-mono">{f.callsign}</span>}>
        Not on the VATSIM network right now. Connect and file a flight plan, then look again.
      </EmptyState>
    );
  }

  const constraints = constraintsOf(f);

  return (
    <div className="flex flex-col gap-6">
      <Card className="flex flex-wrap items-center gap-x-6 gap-y-3 p-4">
        <div className="flex items-center gap-3">
          <span className="flex size-10 items-center justify-center rounded-sm bg-brand-soft text-brand-ink">
            <Plane className="size-5" />
          </span>
          <div className="flex flex-col gap-1">
            <span className="font-mono text-xl font-bold leading-none">{f.callsign}</span>
            <span className="font-mono text-xs text-ink-2">
              {f.aircraft_type} · {f.dep} → {f.arr}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2 font-mono text-xs text-ink-2">
          <StatusPill tone={toneOf("flight", f.status === "airborne" ? "airborne" : "ground")}>
            {f.status === "airborne" ? "airborne" : "on the ground"}
          </StatusPill>
          {f.status === "airborne" && (
            <span>
              FL{Math.round(f.altitude / 100)} · {f.groundspeed}kt
            </span>
          )}
        </div>
        <Link
          to="/advisories/fcas"
          search={{ flight: f.callsign }}
          className={buttonVariants({ variant: "outline", size: "sm", className: "ml-auto" })}
        >
          <Waypoints />
          Show {f.callsign} on the FCA map
        </Link>
      </Card>

      <div className="grid gap-3 sm:grid-cols-3">
        <MetricCard
          label="Predicted delay"
          icon={Clock}
          value={f.total_delay_min > 0 ? `${f.total_delay_min}′` : "—"}
          tone={f.total_delay_min > 0 ? "bad" : undefined}
        />
        <MetricCard label="EDCT" icon={Timer} value={f.edct ? hhmmZulu(f.edct) : "—"} />
        <MetricCard label="Initiatives affecting" icon={TrafficCone} value={constraints.length} />
      </div>

      <section className="flex flex-col gap-3">
        <h2 className="text-xl font-bold">Traffic management</h2>
        <DataTable
          columns={constraintColumns}
          data={constraints}
          getRowId={(r) => r.id}
          rowCap={25}
          label="Initiatives affecting this flight"
          empty="No active traffic management is affecting this flight right now."
        />
      </section>
    </div>
  );
}

export function PilotPage() {
  const [callsign, setCallsign] = useState<string | null>(null);
  const { data: me } = useMe();
  const traffic = useTraffic();
  const flight = usePublicFlight(callsign);
  // Auto-resolve the signed-in pilot's own flight when they haven't searched for a specific one.
  const mine = useMyFlight(!!me && !callsign);
  const board = usePublicBoard();
  const restrictions = board.data?.restrictions ?? [];

  usePageHeader({ subtitle: "Look up how active traffic management is affecting a flight." });

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <FlightSearch
          aircraft={traffic.data ?? []}
          onSelect={setCallsign}
          placeholder="Callsign — e.g. AAL1234"
          autoFocus
          className="w-full max-w-md"
        />
        {!callsign && mine.data?.found && (
          <span className="text-xs text-ink-3">Showing your flight — search to look up another.</span>
        )}
      </div>

      {callsign ? (
        <QueryState
          isLoading={flight.isLoading}
          isError={flight.isError}
          loading={`Looking up ${callsign}…`}
          error={`Couldn’t look up ${callsign}. Try again.`}
        >
          {flight.data && <Result f={flight.data} />}
        </QueryState>
      ) : mine.data?.found ? (
        <Result f={mine.data} />
      ) : me && !mine.isLoading && mine.data ? (
        <EmptyState icon={Plane}>
          You’re not on the VATSIM network under your CID right now. Connect and file a flight plan, or search
          for a callsign above.
        </EmptyState>
      ) : null}

      {restrictions.length > 0 && (
        <section className="flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold">Active restrictions</h2>
            <span className="font-mono text-sm text-ink-3">{restrictions.length}</span>
          </div>
          <DataTable
            columns={restrictionColumns}
            data={restrictions}
            getRowId={(r) => r.id}
            rowCap={10}
            label="Active restrictions"
          />
        </section>
      )}
    </div>
  );
}

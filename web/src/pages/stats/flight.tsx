import {Card, DataTable, type DataColumn, EmptyState, MetricCard, QueryState, StatusPill} from "@ois/ui";
import {useParams} from "@tanstack/react-router";
import {Clock, Gauge, Lock, MountainSnow, Route, Timer} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type StatsFlightDetail, useFlightDetail} from "@/lib/stats";
import {toneOf} from "@/lib/status";
import {formatZuluFull} from "@/lib/time";

type Revision = StatsFlightDetail["revisions"][number];

function Field({ label, value, mono = true }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-ink-2">{label}</span>
      <span className={mono ? "font-mono text-sm tabular-nums" : "text-sm"}>{value}</span>
    </div>
  );
}

function dur(s?: number | null) {
  if (s == null) return "—";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

const REVISION_COLUMNS: DataColumn<Revision>[] = [
  {
    accessorKey: "effective_from",
    header: "Effective",
    icon: Clock,
    mono: true,
    cell: (c) => <span className="whitespace-nowrap text-ink-2">{formatZuluFull(c.getValue<string>())}</span>,
  },
  {
    id: "route",
    accessorFn: (r) => `${r.departure ?? "????"} → ${r.arrival ?? "????"}`,
    header: "Route",
    icon: Route,
    mono: true,
  },
  { id: "aircraft", accessorFn: (r) => r.aircraft_short ?? "—", header: "Aircraft", mono: true },
  {
    id: "kind",
    header: "",
    enableSorting: false,
    cell: (c) => <StatusPill tone="neutral">{c.row.index === 0 ? "filed" : "amended"}</StatusPill>,
  },
];

function Detail({ f }: { f: StatsFlightDetail }) {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <StatusPill tone={toneOf("session", f.status)} dot>
          {f.status}
        </StatusPill>
        <span className="font-mono text-sm text-ink-2">
          {f.departure ?? "????"} → {f.arrival ?? "????"}
        </span>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard label="Duration" icon={Timer} value={dur(f.duration_s)} />
        <MetricCard
          label="Distance"
          icon={Route}
          value={f.distance_nm != null ? `${Math.round(f.distance_nm)} nm` : "—"}
        />
        <MetricCard label="Max altitude" icon={MountainSnow} value={f.max_altitude != null ? `${f.max_altitude} ft` : "—"} />
        <MetricCard
          label="Max groundspeed"
          icon={Gauge}
          value={f.max_groundspeed != null ? `${f.max_groundspeed} kt` : "—"}
        />
      </div>

      <Card className="flex flex-col gap-4 p-5">
        <h2 className="text-xl font-bold">Details</h2>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Field label="Pilot CID" value={f.cid} />
          <Field label="Aircraft" value={f.aircraft_short ?? "—"} />
          <Field label="Cruise" value={f.cruise_alt ? `${f.cruise_alt} ft` : "—"} />
          <Field label="Alternate" value={f.alternate ?? "—"} />
          <Field label="Logon" value={formatZuluFull(f.logon_time)} />
          <Field label="First seen" value={formatZuluFull(f.first_seen)} />
          <Field label="Last seen" value={formatZuluFull(f.last_seen)} />
          <Field label="Server" value={f.server ?? "—"} mono={false} />
        </div>
        {f.route && (
          <div className="flex flex-col gap-1 border-t border-line pt-4">
            <span className="text-xs text-ink-2">Route</span>
            <p className="whitespace-pre-wrap break-words font-mono text-sm">{f.route}</p>
          </div>
        )}
      </Card>

      {f.revisions.length > 1 && (
        <section className="flex flex-col gap-3">
          <h2 className="text-xl font-bold">
            Plan amendments <span className="font-mono text-ink-3">{f.revisions.length - 1}</span>
          </h2>
          <DataTable
            label="Plan amendments"
            columns={REVISION_COLUMNS}
            data={f.revisions}
            rowCap={25}
          />
        </section>
      )}
    </div>
  );
}

export function StatsFlightPage() {
  const { flightId } = useParams({ from: "/admin/historical/flights/$flightId" });
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");
  const flight = useFlightDetail(flightId);

  usePageHeader({ title: flight.data?.callsign });

  return (
    <div className="flex w-full max-w-5xl flex-col gap-6">
      {!canRead ? (
        <EmptyState icon={Lock}>You don&apos;t have access to network statistics.</EmptyState>
      ) : (
        <QueryState
          isLoading={!flight.data && !flight.isError}
          isError={flight.isError}
          error="That flight isn't in the stats database (it may have aged out)."
        >
          {flight.data && <Detail f={flight.data} />}
        </QueryState>
      )}
    </div>
  );
}

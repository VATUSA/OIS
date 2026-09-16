import {Badge, Card, CardContent} from "@ois/ui";
import {Link, useParams} from "@tanstack/react-router";
import {ArrowLeft} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type StatsFlightDetail, useFlightDetail} from "@/lib/stats";
import {formatZuluFull} from "@/lib/time";

function Field({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-sm">{value}</span>
    </div>
  );
}

function dur(s?: number | null) {
  if (s == null) return "—";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function statusVariant(status: string): "success" | "secondary" | "outline" {
  if (status === "completed") return "secondary";
  if (status === "active") return "success";
  return "outline";
}

function Detail({ f }: { f: StatsFlightDetail }) {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">{f.callsign}</h1>
        <Badge variant={statusVariant(f.status)}>{f.status}</Badge>
        <span className="font-mono text-sm text-muted-foreground">
          {f.departure ?? "????"} → {f.arrival ?? "????"}
        </span>
      </div>

      <Card>
        <CardContent className="grid grid-cols-2 gap-4 pt-6 sm:grid-cols-3">
          <Field label="Pilot CID" value={f.cid} />
          <Field label="Aircraft" value={f.aircraft_short ?? "—"} />
          <Field label="Cruise" value={f.cruise_alt ? `${f.cruise_alt} ft` : "—"} />
          <Field label="Logon" value={formatZuluFull(f.logon_time)} />
          <Field label="First seen" value={formatZuluFull(f.first_seen)} />
          <Field label="Last seen" value={formatZuluFull(f.last_seen)} />
          <Field label="Duration" value={dur(f.duration_s)} />
          <Field
            label="Distance"
            value={f.distance_nm != null ? `${Math.round(f.distance_nm)} nm` : "—"}
          />
          <Field label="Max altitude" value={f.max_altitude != null ? `${f.max_altitude} ft` : "—"} />
          <Field
            label="Max groundspeed"
            value={f.max_groundspeed != null ? `${f.max_groundspeed} kt` : "—"}
          />
          <Field label="Alternate" value={f.alternate ?? "—"} />
          <Field label="Server" value={f.server ?? "—"} />
        </CardContent>
      </Card>

      {f.route && (
        <Card>
          <CardContent className="flex flex-col gap-1 pt-6">
            <span className="text-xs text-muted-foreground">Route</span>
            <p className="whitespace-pre-wrap break-words font-mono text-sm">{f.route}</p>
          </CardContent>
        </Card>
      )}

      {f.revisions.length > 1 && (
        <Card>
          <CardContent className="flex flex-col gap-2 pt-6">
            <span className="text-xs text-muted-foreground">
              Plan amendments ({f.revisions.length - 1})
            </span>
            <ul className="flex flex-col gap-1.5">
              {f.revisions.map((r, i) => (
                <li key={i} className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 text-sm">
                  <span className="shrink-0 font-mono text-xs text-muted-foreground">
                    {formatZuluFull(r.effective_from)}
                  </span>
                  <span className="font-mono">
                    {r.departure ?? "????"} → {r.arrival ?? "????"}
                  </span>
                  {r.aircraft_short && (
                    <span className="text-xs text-muted-foreground">{r.aircraft_short}</span>
                  )}
                  <span className="text-xs text-muted-foreground">{i === 0 ? "(filed)" : "(amended)"}</span>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

export function StatsFlightPage() {
  const { flightId } = useParams({ from: "/admin/historical/flights/$flightId" });
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");
  const flight = useFlightDetail(flightId);

  const back = (
    <Link
      to="/admin/historical"
      className="flex w-fit items-center gap-1 text-sm text-muted-foreground transition-colors hover:text-foreground"
    >
      <ArrowLeft className="size-4" /> Network statistics
    </Link>
  );

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      {back}
      {!canRead ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have access to network statistics.
          </CardContent>
        </Card>
      ) : flight.isError ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            That flight isn&apos;t in the stats database (it may have aged out).
          </CardContent>
        </Card>
      ) : !flight.data ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">Loading…</CardContent>
        </Card>
      ) : (
        <Detail f={flight.data} />
      )}
    </div>
  );
}

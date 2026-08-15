import {Link} from "@tanstack/react-router";
import {Badge, Card, CardContent} from "@ois/ui";
import {Gauge, OctagonX, RefreshCw, Split, Timer, Waypoints,} from "lucide-react";

import {
  type PublicGdp,
  type PublicGroundStop,
  type PublicProgram,
  type PublicRestriction,
  usePublicBoard,
} from "@/lib/public";

/** "1423z" from an ISO datetime (Zulu). */
function zulu(iso: string): string {
  const d = new Date(iso);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCHours())}${p(d.getUTCMinutes())}z`;
}

/** "1400z" from a bare HHMM clock string. */
function hhmm(s: string): string {
  return /^\d{4}$/.test(s) ? `${s}z` : s;
}

function ScopeChip({ scope }: { scope: string }) {
  const codes = scope.trim();
  return (
    <Badge variant="outline" className="font-mono">
      {codes === "" ? "field-wide" : codes}
    </Badge>
  );
}

/** Live inbound demand (next 60 min), red when it exceeds the AAR. */
function DemandChip({ demand, over }: { demand: number; over: boolean }) {
  return (
    <Badge variant={over ? "destructive" : "outline"} className="font-mono">
      ↓ {demand}/hr{over ? " over" : ""}
    </Badge>
  );
}

/** A framed list for one initiative type. */
function Section({
  title,
  icon: Icon,
  tone,
  count,
  empty,
  children,
}: {
  title: string;
  icon: typeof Gauge;
  tone?: string;
  count: number;
  empty: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="flex flex-col">
      <CardContent className="flex flex-1 flex-col gap-3 pt-6">
        <div className="flex items-center gap-2">
          <span
            className={
              "flex size-8 items-center justify-center rounded-md bg-primary/10 " +
              (tone ?? "text-primary")
            }
          >
            <Icon className="size-4" />
          </span>
          <span className="text-sm font-semibold">{title}</span>
          <span className="ml-auto text-sm text-muted-foreground tabular-nums">
            {count}
          </span>
        </div>
        {count === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{empty}</p>
        ) : (
          <ul className="flex flex-col divide-y">{children}</ul>
        )}
      </CardContent>
    </Card>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return <li className="flex flex-col gap-1 py-2.5 first:pt-0 last:pb-0">{children}</li>;
}

function GroundStopRow({ gs }: { gs: PublicGroundStop }) {
  return (
    <Row>
      <div className="flex items-center gap-2">
        <span className="font-mono text-sm font-semibold">{gs.airport}</span>
        <ScopeChip scope={gs.scope} />
        <span className="ml-auto font-mono text-xs text-muted-foreground">
          {gs.until ? `until ${hhmm(gs.until)}` : "until further notice"}
        </span>
      </div>
    </Row>
  );
}

function GdpRow({ gdp }: { gdp: PublicGdp }) {
  return (
    <Row>
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-sm font-semibold">{gdp.airport}</span>
        <Badge variant="secondary">AAR {gdp.aar}</Badge>
        <DemandChip demand={gdp.demand_60min} over={gdp.over_capacity} />
        <ScopeChip scope={gdp.scope} />
        {gdp.max_enroute_min != null && (
          <Badge variant="outline">≤{gdp.max_enroute_min}min out</Badge>
        )}
        <span className="ml-auto font-mono text-xs text-muted-foreground">
          {hhmm(gdp.start_time)}–{hhmm(gdp.end_time)}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
        {gdp.controlled > 0 ? (
          <span className="font-mono">
            {gdp.controlled} delayed · avg {gdp.avg_delay_min}′ · max{" "}
            {gdp.max_delay_min}′
          </span>
        ) : (
          <span className="font-mono">no controlled flights</span>
        )}
      </div>
    </Row>
  );
}

function RestrictionRow({ r }: { r: PublicRestriction }) {
  return (
    <Row>
      <span className="text-sm font-medium">{r.restriction}</span>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <span className="font-mono">
          {r.requesting} <span className="text-muted-foreground/60">req</span> ·{" "}
          {r.providing} <span className="text-muted-foreground/60">prov</span>
        </span>
        <span className="ml-auto font-mono">
          {zulu(r.start_time)}
          {r.stop_time ? `–${zulu(r.stop_time)}` : " · UFN"}
        </span>
      </div>
    </Row>
  );
}

function spacing(trail: number, mit: number): string {
  if (mit > 0) return `${mit} MIT`;
  if (trail > 0) return `${trail} MIN`;
  return "AAR only";
}

function ProgramRow({ p }: { p: PublicProgram }) {
  return (
    <Row>
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-sm font-semibold">{p.icao}</span>
        <Badge variant="secondary">AAR {p.aar}</Badge>
        <DemandChip demand={p.demand_60min} over={p.over_capacity} />
        <Badge variant="outline">{spacing(p.trail, p.mit)}</Badge>
        {p.jets_only && <Badge variant="outline">jets only</Badge>}
        {p.gates.length > 0 && (
          <span className="text-xs text-muted-foreground">
            {p.gates.length} gate{p.gates.length === 1 ? "" : "s"}
          </span>
        )}
      </div>
    </Row>
  );
}

export function AdvisoriesPage() {
  const board = usePublicBoard();
  const b = board.data;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Advisories</h1>
          <p className="text-muted-foreground">
            Active traffic management initiatives across the NAS.
          </p>
        </div>
        <div className="flex items-center gap-4">
          <Link
            to="/advisories/fcas"
            className="flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
          >
            <Waypoints className="size-4" />
            FCA overview
          </Link>
          {b && (
            <span className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
              <RefreshCw
                className={`size-3 ${board.isFetching ? "animate-spin" : ""}`}
              />
              updated {zulu(b.as_of)}
            </span>
          )}
        </div>
      </div>

      {board.isLoading ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          Loading advisories…
        </p>
      ) : board.isError ? (
        <p className="py-16 text-center text-sm text-destructive">
          Couldn’t load advisories. Retrying…
        </p>
      ) : (
        <div className="grid gap-4 lg:grid-cols-2">
          <Section
            title="Ground Stops"
            icon={OctagonX}
            tone={
              (b?.ground_stops.length ?? 0) > 0
                ? "text-destructive"
                : "text-primary"
            }
            count={b?.ground_stops.length ?? 0}
            empty="No active ground stops."
          >
            {b?.ground_stops.map((gs) => (
              <GroundStopRow key={gs.id} gs={gs} />
            ))}
          </Section>

          <Section
            title="Ground Delay Programs"
            icon={Timer}
            count={b?.gdps.length ?? 0}
            empty="No active ground delay programs."
          >
            {b?.gdps.map((gdp) => (
              <GdpRow key={gdp.id} gdp={gdp} />
            ))}
          </Section>

          <Section
            title="Restrictions"
            icon={Split}
            count={b?.restrictions.length ?? 0}
            empty="No active restrictions."
          >
            {b?.restrictions.map((r) => (
              <RestrictionRow key={r.id} r={r} />
            ))}
          </Section>

          <Section
            title="Rate Programs"
            icon={Gauge}
            count={b?.programs.length ?? 0}
            empty="No active rate programs."
          >
            {b?.programs.map((p) => (
              <ProgramRow key={p.icao} p={p} />
            ))}
          </Section>
        </div>
      )}
    </div>
  );
}

import {useState} from "react";
import {Link} from "@tanstack/react-router";
import {Badge, Card, CardContent} from "@ois/ui";
import {OctagonX, Plane, Split, Timer, Waypoints} from "lucide-react";

import {FlightSearch} from "@/components/flight-search";
import {useMe} from "@/lib/auth";
import {useTraffic} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";
import {type FlightAdvisory, useMyFlight, usePublicBoard, usePublicFlight} from "@/lib/public";

function DelayBadge({ min }: { min: number }) {
  if (min <= 0) return <Badge variant="outline">no delay</Badge>;
  return <Badge variant="destructive">{min} min delay</Badge>;
}

/** One constraint card in the affecting-initiatives list. */
function Constraint({
  icon: Icon,
  title,
  children,
}: {
  icon: typeof Timer;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3 rounded-lg border p-3">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
        <Icon className="size-4" />
      </span>
      <div className="flex flex-col gap-1">
        <span className="text-sm font-semibold">{title}</span>
        {children}
      </div>
    </div>
  );
}

function Result({ f }: { f: FlightAdvisory }) {
  if (!f.found) {
    return (
      <Card>
        <CardContent className="py-10 text-center text-sm text-muted-foreground">
          <span className="font-mono font-semibold text-foreground">
            {f.callsign}
          </span>{" "}
          isn’t on the VATSIM network right now. Connect and file a flight plan,
          then look again.
        </CardContent>
      </Card>
    );
  }

  const affected =
    !!f.gdp || !!f.ground_stop || !!f.rate_program || f.fcas.length > 0;

  return (
    <div className="flex flex-col gap-4">
      {/* Identity + headline delay */}
      <Card>
        <CardContent className="flex flex-wrap items-center gap-x-6 gap-y-3 pt-6">
          <div className="flex items-center gap-3">
            <span className="flex size-10 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Plane className="size-5" />
            </span>
            <div className="flex flex-col">
              <span className="font-mono text-lg font-semibold leading-none">
                {f.callsign}
              </span>
              <span className="text-xs text-muted-foreground">
                {f.aircraft_type} · {f.dep} → {f.arr}
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
            <Badge variant="secondary">
              {f.status === "airborne" ? "airborne" : "on the ground"}
            </Badge>
            {f.status === "airborne" && (
              <span>
                FL{Math.round(f.altitude / 100)} · {f.groundspeed}kt
              </span>
            )}
          </div>
          <div className="ml-auto flex flex-col items-end">
            <span className="text-2xl font-semibold tabular-nums leading-none">
              {f.total_delay_min > 0 ? `${f.total_delay_min}′` : "—"}
            </span>
            <span className="text-xs uppercase tracking-wide text-muted-foreground">
              {f.edct ? `EDCT ${hhmmZulu(f.edct)}` : "predicted delay"}
            </span>
          </div>
        </CardContent>
      </Card>

      {!affected ? (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            No active traffic management is affecting this flight right now.
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-2">
          {f.gdp && (
            <Constraint icon={Timer} title={`Ground Delay Program · ${f.gdp.airport}`}>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <Badge variant="secondary">AAR {f.gdp.aar}</Badge>
                <span className="font-mono">
                  {f.gdp.start_time}z–{f.gdp.end_time}z
                </span>
                {f.gdp.controlled ? (
                  <span className="font-mono">
                    EDCT {hhmmZulu(f.gdp.edct)} · {f.gdp.delay_min} min delay
                  </span>
                ) : (
                  <span>subject to the program (no slot assigned yet)</span>
                )}
              </div>
            </Constraint>
          )}

          {f.ground_stop && (
            <Constraint
              icon={OctagonX}
              title={`Ground Stop · ${f.ground_stop.airport}`}
            >
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <Badge variant="outline" className="font-mono">
                  {f.ground_stop.scope.trim() === ""
                    ? "field-wide"
                    : f.ground_stop.scope}
                </Badge>
                <span className="font-mono">
                  {f.ground_stop.until
                    ? `until ${f.ground_stop.until}z`
                    : "until further notice"}
                </span>
              </div>
            </Constraint>
          )}

          {f.rate_program && (
            <Constraint
              icon={Split}
              title={`Arrival metering · ${f.rate_program.airport}`}
            >
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <Badge variant="secondary">AAR {f.rate_program.aar}</Badge>
                <DelayBadge min={f.rate_program.delay_min} />
                {f.rate_program.sta && (
                  <span className="font-mono">
                    STA {hhmmZulu(f.rate_program.sta)}
                  </span>
                )}
              </div>
            </Constraint>
          )}

          {f.fcas.map((x) => (
            <Constraint
              key={x.fca_id}
              icon={Waypoints}
              title={`Flow Constrained Area · ${x.fca_name}`}
            >
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span
                  className="size-2.5 rounded-full"
                  style={{ background: x.color }}
                />
                {x.cross_time && (
                  <span className="font-mono">cross {hhmmZulu(x.cross_time)}</span>
                )}
                <DelayBadge min={x.delay_min} />
                {x.edct && (
                  <span className="font-mono">EDCT {hhmmZulu(x.edct)}</span>
                )}
              </div>
            </Constraint>
          ))}
        </div>
      )}

      <Link
        to="/advisories/fcas"
        search={{ flight: f.callsign }}
        className="flex items-center gap-1.5 self-start text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <Waypoints className="size-4" />
        Show {f.callsign} on the FCA map
      </Link>
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

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold">My flight</h1>
        <p className="text-muted-foreground">
          Look up how active traffic management is affecting a flight.
        </p>
      </div>

      <FlightSearch
        aircraft={traffic.data ?? []}
        onSelect={setCallsign}
        placeholder="Callsign — e.g. AAL1234"
        autoFocus
      />

      {callsign ? (
        flight.isLoading ? (
          <p className="py-10 text-center text-sm text-muted-foreground">
            Looking up {callsign}…
          </p>
        ) : flight.isError ? (
          <p className="py-10 text-center text-sm text-destructive">
            Couldn’t look up {callsign}. Try again.
          </p>
        ) : flight.data ? (
          <Result f={flight.data} />
        ) : null
      ) : mine.data?.found ? (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            Showing your flight — search above to look up another.
          </p>
          <Result f={mine.data} />
        </div>
      ) : me && !mine.isLoading && mine.data ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            You’re not on the VATSIM network under your CID right now. Connect and file a flight
            plan, or search for a callsign above.
          </CardContent>
        </Card>
      ) : null}

      {restrictions.length > 0 && (
        <Card>
          <CardContent className="flex flex-col gap-3 pt-6">
            <div className="flex items-center gap-2">
              <Split className="size-4 text-muted-foreground" />
              <span className="text-sm font-semibold">Active restrictions</span>
              <Badge variant="secondary">{restrictions.length}</Badge>
            </div>
            <ul className="flex flex-col divide-y divide-border/60">
              {restrictions.map((r) => (
                <li key={r.id} className="flex flex-col gap-0.5 py-2 text-sm">
                  <span>{r.decoded || r.restriction}</span>
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span className="font-mono">
                      {r.requesting} → {r.providing}
                    </span>
                    <span className="ml-auto font-mono">
                      {hhmmZulu(r.start_time)}
                      {r.stop_time ? `–${hhmmZulu(r.stop_time)}` : " · UFN"}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

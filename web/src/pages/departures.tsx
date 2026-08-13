import {useState} from "react";
import {Badge, Button, Card, CardContent, Input} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {type Departure, useDepartures, useIssueCfr, useReleaseCfr,} from "@/lib/departures";
import {hasPermission} from "@/lib/permissions";
import {hhmmZulu, parseHhmm} from "@/lib/time";

function delayClass(min: number): string {
  if (min >= 15) return "text-destructive";
  if (min > 0) return "text-amber-500";
  return "text-muted-foreground";
}

export function DepartureRow({
  d,
  canIssue,
  leading,
}: {
  d: Departure;
  canIssue: boolean;
  leading: React.ReactNode;
}) {
  const issue = useIssueCfr();
  const release = useReleaseCfr();
  const [ready, setReady] = useState("");
  const busy = issue.isPending || release.isPending;
  const now = Date.now();
  const releaseNow = d.cfr ? new Date(d.cfr).getTime() <= now + 60_000 : false;

  function setReadyTime() {
    const iso = parseHhmm(ready);
    if (!iso) return;
    issue.mutate(
      { callsign: d.callsign, airport: d.arrival, readyTime: iso },
      { onSuccess: () => setReady("") },
    );
  }

  return (
    <tr className="border-t">
      <td className="py-2 pr-3 tabular-nums text-muted-foreground">{leading}</td>
      <td className="py-2 pr-3 font-mono font-medium">{d.callsign}</td>
      <td className="py-2 pr-3 font-mono text-xs">{d.arrival}</td>
      <td className="py-2 pr-3">{d.aircraft_type}</td>
      <td className="py-2 pr-3 font-mono text-xs">{d.gate ?? "—"}</td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {hhmmZulu(d.eta)}
      </td>
      <td className={`py-2 pr-3 text-right tabular-nums ${delayClass(d.delay_min)}`}>
        {d.delay_min > 0 ? `+${d.delay_min}` : "—"}
      </td>
      <td className="py-2 pr-3 text-right">
        {d.cfr ? (
          <span
            className={`font-mono text-xs tabular-nums ${releaseNow ? "text-emerald-500" : "text-amber-500"}`}
          >
            {hhmmZulu(d.cfr)}
          </span>
        ) : (
          "—"
        )}
      </td>
      <td className="py-2 pr-3">
        {d.has_program && !d.cfr_issued && canIssue ? (
          <div className="flex items-center gap-1">
            <Input
              className="h-8 w-20 font-mono text-xs"
              placeholder="HHMMz"
              value={ready}
              onChange={(e) => setReady(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && setReadyTime()}
            />
            <Button
              size="sm"
              variant="secondary"
              disabled={busy || !parseHhmm(ready)}
              onClick={setReadyTime}
            >
              Set
            </Button>
          </div>
        ) : (
          <span className="text-muted-foreground">—</span>
        )}
      </td>
      <td className="py-2 pr-3">
        {!d.has_program ? (
          <Badge variant="outline">no program</Badge>
        ) : d.cfr_issued ? (
          <Badge variant="success">issued</Badge>
        ) : (
          <Badge variant="secondary">proposed</Badge>
        )}
      </td>
      <td className="py-2 text-right">
        {!d.has_program ? (
          <span className="text-xs text-muted-foreground">Release at will</span>
        ) : canIssue ? (
          <div className="flex justify-end gap-1">
            {d.cfr_issued ? (
              <Button
                size="sm"
                variant="ghost"
                className="text-destructive hover:text-destructive"
                disabled={busy}
                onClick={() => release.mutate(d.callsign)}
              >
                Cancel
              </Button>
            ) : (
              <>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() =>
                    issue.mutate({ callsign: d.callsign, airport: d.arrival })
                  }
                >
                  Issue CFR
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    issue.mutate({
                      callsign: d.callsign,
                      airport: d.arrival,
                      readyTime: new Date().toISOString(),
                    })
                  }
                >
                  Release now
                </Button>
              </>
            )}
          </div>
        ) : null}
      </td>
    </tr>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col">
      <span className="text-3xl font-semibold tabular-nums">{value}</span>
      <span className="text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
    </div>
  );
}

export function DeparturesPage() {
  const { data: me } = useMe();
  const [query, setQuery] = useState("");
  const [field, setField] = useState("");
  const departures = useDepartures(field);
  const canIssue = hasPermission(me, "tmu.cfr.assign");

  function load() {
    const clean = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (clean.length >= 3) setField(clean);
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {field ? `${field} departures` : "Departures"}
        </h1>
        <p className="text-muted-foreground">
          Every pending departure out of a field. Ones bound for a metered destination
          get a Call-For-Release (CFR); the rest release at will.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-wrap items-end gap-3 pt-6">
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Departure field
            <Input
              className="w-32 font-mono uppercase"
              maxLength={4}
              placeholder="KBOS"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && load()}
            />
          </label>
          <Button onClick={load}>Load</Button>
          {field && departures.data && (
            <span className="ml-auto text-xs text-muted-foreground">
              {departures.isFetching ? "refreshing…" : "live · updates every 20s"}
            </span>
          )}
        </CardContent>
      </Card>

      {!field ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Enter a departure field to see its departures.
          </CardContent>
        </Card>
      ) : departures.isError ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Couldn&apos;t load departures for {field}.
          </CardContent>
        </Card>
      ) : !departures.data ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Loading {field}…
          </CardContent>
        </Card>
      ) : (
        <>
          <Card>
            <CardContent className="flex flex-col gap-4 pt-6">
              <div className="flex flex-wrap gap-10">
                <Stat label="Total" value={departures.data.total} />
                <Stat label="To metered fields" value={departures.data.to_metered} />
                <Stat label="Holding on CFR" value={departures.data.holding_on_cfr} />
              </div>
              <p className="text-sm text-muted-foreground">
                Destinations with a TMU program:{" "}
                {departures.data.program_destinations.length ? (
                  <span className="font-mono text-foreground">
                    {departures.data.program_destinations.join(", ")}
                  </span>
                ) : (
                  "none"
                )}
              </p>
            </CardContent>
          </Card>

          <Card>
            <CardContent className="pt-6">
              {departures.data.departures.length === 0 ? (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  No pending departures out of {field}.
                </p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <th className="pb-2 pr-3 text-right font-medium">#</th>
                        <th className="pb-2 pr-3 font-medium">Callsign</th>
                        <th className="pb-2 pr-3 font-medium">To</th>
                        <th className="pb-2 pr-3 font-medium">Type</th>
                        <th className="pb-2 pr-3 font-medium">Gate</th>
                        <th className="pb-2 pr-3 font-medium">ETA</th>
                        <th className="pb-2 pr-3 text-right font-medium">Delay</th>
                        <th className="pb-2 pr-3 text-right font-medium">CFR</th>
                        <th className="pb-2 pr-3 font-medium">Ready</th>
                        <th className="pb-2 pr-3 font-medium" />
                        <th className="pb-2" />
                      </tr>
                    </thead>
                    <tbody>
                      {departures.data.departures.map((d) => (
                        <DepartureRow
                          key={d.callsign}
                          d={d}
                          canIssue={canIssue}
                          leading={d.seq ?? "—"}
                        />
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}

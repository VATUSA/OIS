import {useState} from "react";
import {Badge, Button, Card, CardContent, Input} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {type Departure, useDepartures, useIssueCfr, useReleaseCfr,} from "@/lib/departures";
import {hasPermission} from "@/lib/permissions";
import {hhmmZulu} from "@/lib/time";

function delayClass(min: number): string {
  if (min >= 15) return "text-destructive";
  if (min > 0) return "text-amber-500";
  return "text-muted-foreground";
}

function DepartureRow({ d, canIssue }: { d: Departure; canIssue: boolean }) {
  const issue = useIssueCfr();
  const release = useReleaseCfr();
  const busy = issue.isPending || release.isPending;
  const now = Date.now();
  const releaseNow = d.cfr ? new Date(d.cfr).getTime() <= now + 60_000 : false;

  return (
    <tr className="border-t">
      <td className="py-2 pr-3 text-right tabular-nums text-muted-foreground">
        {d.seq ?? "—"}
      </td>
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
        {d.cfr_issued ? (
          <Badge variant="success">issued</Badge>
        ) : (
          <Badge variant="secondary">proposed</Badge>
        )}
      </td>
      <td className="py-2 text-right">
        {canIssue && (
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
        )}
      </td>
    </tr>
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
        <h1 className="text-2xl font-semibold tracking-tight">Departures</h1>
        <p className="text-muted-foreground">
          Pending departures out of a field into any metered destination, with their
          Call-For-Release (CFR) times.
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
            Enter a departure field to see its metered departures.
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="pt-6">
            {departures.isError ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                Couldn&apos;t load departures for {field}.
              </p>
            ) : !departures.data ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                Loading…
              </p>
            ) : departures.data.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                No pending departures out of {field} into a metered field.
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
                      <th className="pb-2 pr-3 font-medium" />
                      <th className="pb-2" />
                    </tr>
                  </thead>
                  <tbody>
                    {departures.data.map((d) => (
                      <DepartureRow key={d.callsign} d={d} canIssue={canIssue} />
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}

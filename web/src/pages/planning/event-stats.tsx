import {useEffect, useState} from "react";
import {Badge, Button, buttonVariants, Card, CardContent, Input} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {BarChart3, Circle, Film, Plane} from "lucide-react";

import {type EventCapture, type EventStats, useEventCapture, useEventStats, useUpdateEventCapture,} from "@/lib/event-stats";
import {
  type AirportRate,
  type FacilitySupport,
  type TmiPackage,
  useAirportRates,
  useEventDebrief,
  useFacilitySupport,
  usePackages,
  useUpdateEventDebrief,
} from "@/lib/events";
import {formatZuluFull, timeAgo} from "@/lib/time";

const clampMin = (n: number) => Math.max(0, Math.min(720, Math.round(n)));

function statusBadge(cap: EventCapture) {
  if (cap.capture_status === "open")
    return (
      <Badge variant="success" className="gap-1">
        <Circle className="size-2.5 animate-pulse fill-current" />
        recording
      </Badge>
    );
  if (cap.capture_status === "saved") return <Badge variant="secondary">saved</Badge>;
  if (cap.enabled) return <Badge variant="outline">scheduled</Badge>;
  return <Badge variant="outline">off</Badge>;
}

function CaptureConfig({ eventId }: { eventId: number }) {
  const capture = useEventCapture(eventId);
  const update = useUpdateEventCapture(eventId);
  const cap = capture.data;
  const [pre, setPre] = useState("30");
  const [post, setPost] = useState("30");

  useEffect(() => {
    if (cap) {
      setPre(String(cap.pre_minutes));
      setPost(String(cap.post_minutes));
    }
  }, [cap?.pre_minutes, cap?.post_minutes]);

  if (!cap) return <p className="py-2 text-sm text-muted-foreground">Loading…</p>;
  const canEdit = cap.can_edit;

  const save = (patch: { enabled?: boolean; pre?: number; post?: number }) =>
    update.mutate({
      enabled: patch.enabled ?? cap.enabled,
      pre_minutes: patch.pre ?? clampMin(Number(pre) || 0),
      post_minutes: patch.post ?? clampMin(Number(post) || 0),
    });

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
              <BarChart3 className="size-4" />
            </span>
            <div className="flex flex-col">
              <span className="font-semibold">Stats capture</span>
              <span className="text-xs text-muted-foreground">
                Collect and permanently keep this event&apos;s network traffic for its debrief.
              </span>
            </div>
          </div>
          {statusBadge(cap)}
        </div>

        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={cap.enabled}
            disabled={!canEdit || update.isPending}
            onChange={(e) => save({ enabled: e.target.checked })}
          />
          Collect stats for this event
        </label>

        <div className="flex flex-wrap gap-4">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Start margin (min before)</span>
            <Input
              className="h-8 w-28 tabular-nums"
              type="number"
              min={0}
              max={720}
              value={pre}
              disabled={!canEdit}
              onChange={(e) => setPre(e.target.value)}
              onBlur={() => save({ pre: clampMin(Number(pre) || 0) })}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">End margin (min after)</span>
            <Input
              className="h-8 w-28 tabular-nums"
              type="number"
              min={0}
              max={720}
              value={post}
              disabled={!canEdit}
              onChange={(e) => setPost(e.target.value)}
              onBlur={() => save({ post: clampMin(Number(post) || 0) })}
            />
          </label>
        </div>

        {cap.capture_start && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">
              Window {formatZuluFull(cap.capture_start)} –{" "}
              {cap.capture_end ? formatZuluFull(cap.capture_end) : "now (recording)"}
            </p>
            {cap.capture_id && (
              <Link
                to="/admin/historical/replay"
                search={{ capture: cap.capture_id }}
                className={buttonVariants({ variant: "outline", size: "sm" }) + " gap-1"}
              >
                <Film className="size-3.5" />
                Replay on map
              </Link>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

type AirportStat = EventStats["airports"][number];
type CombinedStat = EventStats["combined"];

/** A headline number + label (used for the combined totals). */
function Tile({ label, value, icon }: { label: string; value: string; icon?: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1 rounded-md border bg-muted/20 p-3">
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className="text-2xl font-semibold tabular-nums">{value}</span>
    </div>
  );
}

function AircraftBadges({ items }: { items: { key?: string | null; count: number }[] }) {
  if (items.length === 0) return <span className="text-xs text-muted-foreground/70">No data</span>;
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map((a) => (
        <Badge key={a.key ?? "?"} variant="secondary" className="gap-1 font-mono text-xs">
          {a.key ?? "?"}
          <span className="text-muted-foreground">{a.count}</span>
        </Badge>
      ))}
    </div>
  );
}

/** A compact stat used inside an airport card. */
function MiniStat({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col">
      <span className="text-xl font-semibold tabular-nums">{value}</span>
      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</span>
    </div>
  );
}

/** One featured airport's debrief card. */
function AirportCard({ a }: { a: AirportStat }) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border bg-muted/20 p-4">
      <div className="flex items-baseline justify-between gap-2">
        <span className="font-mono text-base font-semibold">{a.icao}</span>
        <span className="text-xs text-muted-foreground">
          {a.movements} movement{a.movements === 1 ? "" : "s"}
        </span>
      </div>
      <div className="grid grid-cols-3 gap-2">
        <MiniStat label="Arrivals" value={a.arrivals} />
        <MiniStat label="Departures" value={a.departures} />
        <MiniStat label="Pilots" value={a.unique_pilots} />
      </div>
      <div>
        <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
          Top aircraft
        </div>
        <AircraftBadges items={a.top_aircraft} />
      </div>
    </div>
  );
}

/** The featured airports combined into one totals block. */
function CombinedBlock({ c }: { c: CombinedStat }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Tile label="Movements" value={String(c.movements)} icon={<Plane className="size-3" />} />
        <Tile label="Arrivals" value={String(c.arrivals)} />
        <Tile label="Departures" value={String(c.departures)} />
        <Tile label="Unique pilots" value={String(c.unique_pilots)} />
      </div>
      <div>
        <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
          Top aircraft
        </div>
        <AircraftBadges items={c.top_aircraft} />
      </div>
    </div>
  );
}

/** Message shown while there's no debrief yet (before the capture is saved). */
function DebriefPending({ cap }: { cap?: EventCapture }) {
  const msg =
    cap?.capture_status === "open"
      ? "Capture is recording. The debrief is generated once the event ends and its capture is saved."
      : cap?.enabled
        ? "Stats capture is scheduled. The debrief will be generated automatically after the event ends."
        : "Enable stats capture above, and the debrief will be generated automatically after the event.";
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-1 py-12 text-center">
        <span className="flex size-9 items-center justify-center rounded-md bg-muted text-muted-foreground">
          <BarChart3 className="size-4" />
        </span>
        <span className="mt-1 font-medium">Debrief pending</span>
        <span className="max-w-md text-sm text-muted-foreground">{msg}</span>
      </CardContent>
    </Card>
  );
}

/** Planned AAR/ADR (from the event's airport rates) beside what actually flew. */
function PlannedVsActual({
  rates,
  airports,
}: {
  rates: AirportRate[];
  airports: AirportStat[];
}) {
  const byIcao = new Map(airports.map((a) => [a.icao, a]));
  const rows = rates.filter((r) => byIcao.has(r.icao));
  if (rows.length === 0) return null;
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Planned vs. actual</h3>
      <div className="overflow-x-auto rounded-md border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-xs text-muted-foreground">
              <th className="px-3 py-2 font-medium">Airport</th>
              <th className="px-3 py-2 text-right font-medium">Planned AAR</th>
              <th className="px-3 py-2 text-right font-medium">Actual arrivals</th>
              <th className="px-3 py-2 text-right font-medium">Planned ADR</th>
              <th className="px-3 py-2 text-right font-medium">Actual departures</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => {
              const a = byIcao.get(r.icao)!;
              return (
                <tr key={r.icao} className="border-b last:border-0">
                  <td className="px-3 py-2 font-mono">{r.icao}</td>
                  <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{r.aar}</td>
                  <td className="px-3 py-2 text-right font-medium tabular-nums">{a.arrivals}</td>
                  <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{r.adr}</td>
                  <td className="px-3 py-2 text-right font-medium tabular-nums">{a.departures}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">
        Planned rates are per-hour; actuals are totals over the capture window.
      </p>
    </section>
  );
}

function Debrief({ eventId }: { eventId: number }) {
  const capture = useEventCapture(eventId);
  const saved = capture.data?.capture_status === "saved";
  const stats = useEventStats(eventId, saved);
  const rates = useAirportRates(eventId);
  const s = stats.data;

  // The debrief only exists once the event has ended and its capture was saved.
  if (!saved) return <DebriefPending cap={capture.data} />;
  if (!s || !s.captured) {
    return (
      <Card>
        <CardContent className="py-12 text-center text-sm text-muted-foreground">
          Generating debrief…
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-5 pt-6">
        <div className="flex items-center justify-between gap-2">
          <span className="font-semibold">Event debrief</span>
          {s.window_start && s.window_end && (
            <span className="text-xs text-muted-foreground">
              {formatZuluFull(s.window_start)} – {formatZuluFull(s.window_end)}
            </span>
          )}
        </div>

        {s.airports.length === 0 ? (
          <p className="py-4 text-sm text-muted-foreground">
            No featured airports configured for this event — add airport rates to see a debrief.
          </p>
        ) : (
          <>
            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-medium">
                All featured airports
                <span className="ml-2 font-normal text-muted-foreground">
                  {s.airports.map((a) => a.icao).join(" · ")}
                </span>
              </h3>
              <CombinedBlock c={s.combined} />
            </section>

            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-medium">By airport</h3>
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {s.airports.map((a) => (
                  <AirportCard key={a.icao} a={a} />
                ))}
              </div>
            </section>

            <PlannedVsActual rates={rates.data ?? []} airports={s.airports} />
          </>
        )}
      </CardContent>
    </Card>
  );
}

/** A recap of what was planned/coordinated for the event — the other half of the debrief. */
function Coordination({ eventId }: { eventId: number }) {
  const rates = useAirportRates(eventId);
  const packages = usePackages(eventId);
  const support = useFacilitySupport(eventId);

  const airports: AirportRate[] = rates.data ?? [];
  const activated: TmiPackage[] = (packages.data ?? []).filter(
    (p) => p.status === "activated" || p.status === "archived",
  );
  const stored: FacilitySupport[] = (support.data ?? []).filter((f) => f.stored);

  if (airports.length === 0 && activated.length === 0 && stored.length === 0) return null;

  const none = <span className="text-xs text-muted-foreground/70">None</span>;
  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <span className="font-semibold">Coordination</span>
        <div className="grid gap-4 sm:grid-cols-3">
          <div>
            <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
              Featured airports
            </div>
            {airports.length === 0 ? (
              none
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {airports.map((a) => (
                  <Badge key={a.icao} variant="secondary" className="font-mono text-xs">
                    {a.icao}
                    <span className="ml-1 text-muted-foreground">
                      {a.aar}/{a.adr}
                    </span>
                  </Badge>
                ))}
              </div>
            )}
          </div>
          <div>
            <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
              TMI packages run
            </div>
            {activated.length === 0 ? (
              none
            ) : (
              <ul className="flex flex-col gap-0.5 text-xs">
                {activated.map((p) => (
                  <li key={p.id}>
                    <span className="font-medium">{p.name}</span>
                    <span className="text-muted-foreground">
                      {" "}
                      · {p.items.length} item{p.items.length === 1 ? "" : "s"} · {p.status}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div>
            <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
              Facilities
            </div>
            {stored.length === 0 ? (
              none
            ) : (
              <ul className="flex flex-col gap-0.5 text-xs">
                {stored.map((f) => (
                  <li key={f.facility}>
                    <span className="font-mono">{f.facility}</span>
                    <span className="text-muted-foreground">
                      {" "}
                      · {f.level}
                      {f.is_host ? " · host" : ""}
                      {f.has_staffing ? " · staffing" : ""}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

/** Free-text post-event debrief notes (editable with `events.debrief.create`). */
function DebriefNotes({ eventId }: { eventId: number }) {
  const debrief = useEventDebrief(eventId);
  const update = useUpdateEventDebrief(eventId);
  const d = debrief.data;
  const [notes, setNotes] = useState("");
  const [dirty, setDirty] = useState(false);

  // Seed from the server whenever it changes and the user hasn't started editing.
  useEffect(() => {
    if (d && !dirty) setNotes(d.notes);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d?.notes]);

  if (!d) return null;

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex items-center justify-between gap-2">
          <span className="font-semibold">Debrief notes</span>
          {d.updated_at && (
            <span className="text-xs text-muted-foreground">
              {d.updated_by ? `${d.updated_by} · ` : ""}
              {timeAgo(d.updated_at)}
            </span>
          )}
        </div>
        {d.editable ? (
          <>
            <textarea
              className="min-h-[8rem] w-full rounded-md border bg-background p-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              value={notes}
              onChange={(e) => {
                setNotes(e.target.value);
                setDirty(true);
              }}
              placeholder="What went well, what to change next time, notable issues…"
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                disabled={!dirty || update.isPending}
                onClick={() => update.mutate(notes, { onSuccess: () => setDirty(false) })}
              >
                Save
              </Button>
              {dirty && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setNotes(d.notes);
                    setDirty(false);
                  }}
                >
                  Reset
                </Button>
              )}
            </div>
          </>
        ) : d.notes ? (
          <p className="whitespace-pre-wrap text-sm">{d.notes}</p>
        ) : (
          <p className="text-sm text-muted-foreground">No debrief written yet.</p>
        )}
      </CardContent>
    </Card>
  );
}

export function EventStatsSection({ eventId }: { eventId: number }) {
  return (
    <div className="flex flex-col gap-4">
      <CaptureConfig eventId={eventId} />
      <Coordination eventId={eventId} />
      <Debrief eventId={eventId} />
      <DebriefNotes eventId={eventId} />
    </div>
  );
}

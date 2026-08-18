import {useEffect, useState} from "react";
import {Badge, buttonVariants, Card, CardContent, Input} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {BarChart3, Circle, Film, Plane} from "lucide-react";

import {type EventCapture, type EventStats, useEventCapture, useEventStats, useUpdateEventCapture,} from "@/lib/event-stats";
import {formatZuluFull} from "@/lib/time";

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
                to="/historical/replay"
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

function Debrief({ eventId }: { eventId: number }) {
  const capture = useEventCapture(eventId);
  const saved = capture.data?.capture_status === "saved";
  const stats = useEventStats(eventId, saved);
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
          </>
        )}
      </CardContent>
    </Card>
  );
}

export function EventStatsSection({ eventId }: { eventId: number }) {
  return (
    <div className="flex flex-col gap-4">
      <CaptureConfig eventId={eventId} />
      <Debrief eventId={eventId} />
    </div>
  );
}

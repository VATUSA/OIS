import {useEffect, useState} from "react";
import {Badge, buttonVariants, Card, CardContent, Input} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {BarChart3, Circle, Film, Plane, Radio, Users} from "lucide-react";

import {type EventCapture, useEventCapture, useEventStats, useUpdateEventCapture,} from "@/lib/event-stats";
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
                to="/stats/captures/$captureId/replay"
                params={{ captureId: cap.capture_id }}
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

function Tile({ label, value, icon }: { label: string; value: string; icon?: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1 rounded-md border bg-muted/20 p-3">
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className="text-xl font-semibold tabular-nums">{value}</span>
    </div>
  );
}

function StatsReadout({ eventId }: { eventId: number }) {
  const capture = useEventCapture(eventId);
  const live = capture.data?.capture_status === "open";
  const stats = useEventStats(eventId, live);
  const s = stats.data;

  if (!s) return null;
  if (!s.captured) {
    return (
      <Card>
        <CardContent className="py-10 text-center text-sm text-muted-foreground">
          No stats captured yet. Enable capture above — figures appear once the event window opens.
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-5 pt-6">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Tile label="Unique pilots" value={String(s.unique_pilots)} icon={<Plane className="size-3" />} />
          <Tile label="Peak pilots" value={s.peak_pilots == null ? "—" : String(s.peak_pilots)} />
          <Tile label="Arrivals" value={String(s.total_arrivals)} />
          <Tile label="Departures" value={String(s.total_departures)} />
          <Tile
            label="Controllers"
            value={String(s.unique_controllers)}
            icon={<Radio className="size-3" />}
          />
          <Tile label="Positions" value={String(s.controller_positions)} />
          <Tile label="Controller hours" value={s.controller_hours.toFixed(1)} icon={<Users className="size-3" />} />
        </div>

        <div className="grid gap-5 md:grid-cols-2">
          <div>
            <h3 className="mb-2 text-sm font-medium">Airport movements</h3>
            {s.airports.length === 0 ? (
              <p className="text-sm text-muted-foreground">No configured airports.</p>
            ) : (
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-1 pr-3 font-medium">Airport</th>
                    <th className="pb-1 pr-3 font-medium">Arr</th>
                    <th className="pb-1 font-medium">Dep</th>
                  </tr>
                </thead>
                <tbody>
                  {s.airports.map((a) => (
                    <tr key={a.icao} className="border-t">
                      <td className="py-1.5 pr-3 font-mono">{a.icao}</td>
                      <td className="py-1.5 pr-3 tabular-nums">{a.arrivals}</td>
                      <td className="py-1.5 tabular-nums">{a.departures}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>

          <div>
            <h3 className="mb-2 text-sm font-medium">Top aircraft</h3>
            {s.top_aircraft.length === 0 ? (
              <p className="text-sm text-muted-foreground">No data yet.</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {s.top_aircraft.map((a) => (
                  <Badge key={a.key ?? "?"} variant="secondary" className="gap-1 font-mono">
                    {a.key ?? "?"}
                    <span className="text-muted-foreground">{a.count}</span>
                  </Badge>
                ))}
              </div>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function EventStatsSection({ eventId }: { eventId: number }) {
  return (
    <div className="flex flex-col gap-4">
      <CaptureConfig eventId={eventId} />
      <StatsReadout eventId={eventId} />
    </div>
  );
}

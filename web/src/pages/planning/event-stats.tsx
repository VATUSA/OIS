import {useEffect, useMemo, useState} from "react";
import {Button, buttonVariants, Card, type DataColumn, DataTable, EmptyState, Input, MetricCard, StatusPill, Switch, Textarea} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowDownToLine, ArrowUpFromLine, BarChart3, Film, Loader2, Plane, Users} from "lucide-react";

import {type EventCapture, type EventStats, useEventCapture, useEventStats, useUpdateEventCapture} from "@/lib/event-stats";
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
import {toneOf} from "@/lib/status";
import {formatZuluFull, timeAgo} from "@/lib/time";
import {SectionHeader} from "@/pages/planning/section-header";

const clampMin = (n: number) => Math.max(0, Math.min(720, Math.round(n)));

function CapturePill({ cap }: { cap: EventCapture }) {
  if (cap.capture_status === "open")
    return (
      <StatusPill tone={toneOf("recording", "recording")} dot className="[&>span:first-child]:animate-pulse">
        recording
      </StatusPill>
    );
  if (cap.capture_status === "saved") return <StatusPill tone={toneOf("recording", "recorded")}>saved</StatusPill>;
  if (cap.enabled) return <StatusPill tone={toneOf("recording", "scheduled")}>scheduled</StatusPill>;
  return <StatusPill tone="neutral">off</StatusPill>;
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

  if (!cap) return <EmptyState icon={Loader2}>Loading…</EmptyState>;
  const canEdit = cap.can_edit;

  const save = (patch: { enabled?: boolean; pre?: number; post?: number }) =>
    update.mutate({
      enabled: patch.enabled ?? cap.enabled,
      pre_minutes: patch.pre ?? clampMin(Number(pre) || 0),
      post_minutes: patch.post ?? clampMin(Number(post) || 0),
    });

  return (
    <Card className="flex flex-col gap-4 p-4">
      <SectionHeader
        title="Stats capture"
        description="Collect and permanently keep this event's network traffic for its debrief."
        actions={<CapturePill cap={cap} />}
      />

      <div className="flex flex-wrap items-end gap-6">
        <label className="flex items-center gap-2 text-sm">
          <Switch
            checked={cap.enabled}
            disabled={!canEdit || update.isPending}
            onCheckedChange={(v) => save({ enabled: v })}
          />
          Collect stats for this event
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="font-semibold text-ink-2">Start margin (min before)</span>
          <Input
            className="h-8 w-28 font-mono"
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
          <span className="font-semibold text-ink-2">End margin (min after)</span>
          <Input
            className="h-8 w-28 font-mono"
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
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line-soft pt-3">
          <p className="text-xs text-ink-3">
            Window <span className="font-mono">{formatZuluFull(cap.capture_start)}</span> –{" "}
            {cap.capture_end ? <span className="font-mono">{formatZuluFull(cap.capture_end)}</span> : "now (recording)"}
          </p>
          {cap.capture_id && (
            <Link
              to="/admin/historical/replay"
              search={{ capture: cap.capture_id }}
              className={buttonVariants({ variant: "outline", size: "sm" })}
            >
              <Film className="size-3.5" />
              Replay on map
            </Link>
          )}
        </div>
      )}
    </Card>
  );
}

type AirportStat = EventStats["airports"][number];
type CombinedStat = EventStats["combined"];

function AircraftPills({ items }: { items: { key?: string | null; count: number }[] }) {
  if (items.length === 0) return <span className="text-xs text-ink-3">No data</span>;
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map((a) => (
        <StatusPill key={a.key ?? "?"} tone="neutral" className="font-mono">
          {a.key ?? "?"}
          <span className="text-ink-3">{a.count}</span>
        </StatusPill>
      ))}
    </div>
  );
}

const CAPTION = "mb-1.5 text-xs font-semibold text-ink-3";

/** The featured airports combined into one totals block. */
function CombinedBlock({ c }: { c: CombinedStat }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <MetricCard label="Movements" icon={Plane} value={c.movements} />
        <MetricCard label="Arrivals" icon={ArrowDownToLine} value={c.arrivals} />
        <MetricCard label="Departures" icon={ArrowUpFromLine} value={c.departures} />
        <MetricCard label="Unique pilots" icon={Users} value={c.unique_pilots} />
      </div>
      <div>
        <div className={CAPTION}>Top aircraft</div>
        <AircraftPills items={c.top_aircraft} />
      </div>
    </div>
  );
}

const AIRPORT_COLUMNS: DataColumn<AirportStat>[] = [
  { accessorKey: "icao", header: "Airport", icon: Plane, mono: true, cellClassName: "font-semibold" },
  { accessorKey: "movements", header: "Movements", mono: true, align: "right" },
  { accessorKey: "arrivals", header: "Arrivals", icon: ArrowDownToLine, mono: true, align: "right" },
  { accessorKey: "departures", header: "Departures", icon: ArrowUpFromLine, mono: true, align: "right" },
  { accessorKey: "unique_pilots", header: "Pilots", icon: Users, mono: true, align: "right" },
  {
    id: "top_aircraft",
    header: "Top aircraft",
    enableSorting: false,
    cell: (c) => <AircraftPills items={c.row.original.top_aircraft} />,
  },
];

/** Message shown while there's no debrief yet (before the capture is saved). */
function DebriefPending({ cap }: { cap?: EventCapture }) {
  const msg =
    cap?.capture_status === "open"
      ? "Capture is recording. The debrief is generated once the event ends and its capture is saved."
      : cap?.enabled
        ? "Stats capture is scheduled. The debrief will be generated automatically after the event ends."
        : "Enable stats capture above, and the debrief will be generated automatically after the event.";
  return (
    <EmptyState icon={BarChart3} title="Debrief pending" className="rounded-md border border-line py-12">
      {msg}
    </EmptyState>
  );
}

type PlannedRow = { icao: string; aar: number; arrivals: number; adr: number; departures: number };

const PLANNED_COLUMNS: DataColumn<PlannedRow>[] = [
  { accessorKey: "icao", header: "Airport", icon: Plane, mono: true },
  { accessorKey: "aar", header: "Planned AAR", mono: true, align: "right", cellClassName: "text-ink-2" },
  { accessorKey: "arrivals", header: "Actual arrivals", mono: true, align: "right", cellClassName: "font-semibold" },
  { accessorKey: "adr", header: "Planned ADR", mono: true, align: "right", cellClassName: "text-ink-2" },
  { accessorKey: "departures", header: "Actual departures", mono: true, align: "right", cellClassName: "font-semibold" },
];

/** Planned AAR/ADR (from the event's airport rates) beside what actually flew. */
function PlannedVsActual({ rates, airports }: { rates: AirportRate[]; airports: AirportStat[] }) {
  const rows = useMemo(() => {
    const byIcao = new Map(airports.map((a) => [a.icao, a]));
    return rates.flatMap((r) => {
      const a = byIcao.get(r.icao);
      return a ? [{ icao: r.icao, aar: r.aar, arrivals: a.arrivals, adr: r.adr, departures: a.departures }] : [];
    });
  }, [rates, airports]);
  if (rows.length === 0) return null;
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-semibold">Planned vs. actual</h3>
      <DataTable label="Planned vs. actual" columns={PLANNED_COLUMNS} data={rows} getRowId={(r) => r.icao} rowCap={25} />
      <p className="text-xs text-ink-3">Planned rates are per-hour; actuals are totals over the capture window.</p>
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
      <EmptyState icon={Loader2} className="rounded-md border border-line py-12">
        Generating debrief…
      </EmptyState>
    );
  }

  return (
    <Card className="flex flex-col gap-5 p-4">
      <SectionHeader
        title="Event debrief"
        actions={
          s.window_start &&
          s.window_end && (
            <span className="font-mono text-xs text-ink-3">
              {formatZuluFull(s.window_start)} – {formatZuluFull(s.window_end)}
            </span>
          )
        }
      />

      {s.airports.length === 0 ? (
        <p className="py-4 text-sm text-ink-2">
          No featured airports configured for this event — add airport rates to see a debrief.
        </p>
      ) : (
        <>
          <section className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold">
              All featured airports
              <span className="ml-2 font-mono font-normal text-ink-3">{s.airports.map((a) => a.icao).join(" · ")}</span>
            </h3>
            <CombinedBlock c={s.combined} />
          </section>

          <section className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold">By airport</h3>
            <DataTable
              label="By airport"
              columns={AIRPORT_COLUMNS}
              data={s.airports}
              getRowId={(a) => a.icao}
              initialSort={[{ id: "movements", desc: true }]}
              rowCap={25}
            />
          </section>

          <PlannedVsActual rates={rates.data ?? []} airports={s.airports} />
        </>
      )}
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

  const none = <span className="text-xs text-ink-3">None</span>;
  return (
    <Card className="flex flex-col gap-4 p-4">
      <SectionHeader title="Coordination" />
      <div className="grid gap-4 sm:grid-cols-3">
        <div>
          <div className={CAPTION}>Featured airports</div>
          {airports.length === 0 ? (
            none
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {airports.map((a) => (
                <StatusPill key={a.icao} tone="neutral" className="font-mono">
                  {a.icao}
                  <span className="text-ink-3">
                    {a.aar}/{a.adr}
                  </span>
                </StatusPill>
              ))}
            </div>
          )}
        </div>
        <div>
          <div className={CAPTION}>TMI packages run</div>
          {activated.length === 0 ? (
            none
          ) : (
            <ul className="flex flex-col gap-0.5 text-xs">
              {activated.map((p) => (
                <li key={p.id}>
                  <span className="font-semibold">{p.name}</span>
                  <span className="text-ink-3">
                    {" "}
                    · {p.items.length} item{p.items.length === 1 ? "" : "s"} · {p.status}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div>
          <div className={CAPTION}>Facilities</div>
          {stored.length === 0 ? (
            none
          ) : (
            <ul className="flex flex-col gap-0.5 text-xs">
              {stored.map((f) => (
                <li key={f.facility}>
                  <span className="font-mono">{f.facility}</span>
                  <span className="text-ink-3">
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
    <Card className="flex flex-col gap-3 p-4">
      <SectionHeader
        title="Debrief notes"
        actions={
          d.updated_at && (
            <span className="text-xs text-ink-3">
              {d.updated_by ? `${d.updated_by} · ` : ""}
              {timeAgo(d.updated_at)}
            </span>
          )
        }
      />
      {d.editable ? (
        <>
          <Textarea
            className="min-h-32"
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
        <p className="text-sm text-ink-2">No debrief written yet.</p>
      )}
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

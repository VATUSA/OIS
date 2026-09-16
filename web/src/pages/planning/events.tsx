import {useMemo, useState} from "react";
import {
  Button,
  Card,
  type DataColumn,
  DataTable,
  EmptyState,
  FilterBar,
  QueryState,
  SegmentedControl,
  StatusPill,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ois/ui";
import {useNavigate} from "@tanstack/react-router";
import {Building2, CalendarClock, CircleDot, ExternalLink, Lock, Radio, SlidersHorizontal, Type} from "lucide-react";

import {usePageHeader, useView} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {type EventSummary, useUpcomingEvents, vatusaEditUrl} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {formatZuluFull} from "@/lib/time";

const RECORDING_LABEL: Record<string, string> = {
  recording: "Recording",
  scheduled: "Scheduled",
  recorded: "Recorded",
};

type Scope = "upcoming" | "past" | "all";
const SCOPES = [
  { value: "upcoming", label: "Upcoming" },
  { value: "past", label: "Past" },
  { value: "all", label: "All" },
] as const;

const SUBTITLE =
  "VATUSA events. Open one to plan its TMI package, DCC support, facility staffing, and airport rates.";

function endMs(e: EventSummary): number {
  const t = new Date(e.end_time).getTime();
  return Number.isNaN(t) ? 0 : t;
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, " ");

function ReviewPill({ status }: { status: string }) {
  if (!status) return <span className="text-ink-3">—</span>;
  return <StatusPill tone={toneOf("review", status)}>{cap(status)}</StatusPill>;
}

function RecordingPill({ status }: { status: string }) {
  const label = RECORDING_LABEL[status];
  if (!label) return <span className="text-ink-3">—</span>;
  return <StatusPill tone={toneOf("recording", status)}>{label}</StatusPill>;
}

function EditLink({ event }: { event: EventSummary }) {
  const editUrl = vatusaEditUrl(event);
  if (!editUrl) return null;
  return (
    <a
      href={editUrl}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => e.stopPropagation()}
      title="Edit on VATUSA"
      className="rounded-full p-1.5 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
    >
      <ExternalLink className="size-4" />
    </a>
  );
}

function ManageButton({ event }: { event: EventSummary }) {
  const navigate = useNavigate();
  return (
    <Button
      size="sm"
      onClick={(e) => {
        e.stopPropagation();
        navigate({ to: "/admin/planning/events/$eventId", params: { eventId: String(event.id) } });
      }}
    >
      <SlidersHorizontal className="size-3.5" />
      Manage
    </Button>
  );
}

const COLUMNS: DataColumn<EventSummary>[] = [
  {
    accessorKey: "title",
    header: "Name",
    icon: Type,
    cell: (c) => {
      const title = c.getValue<string>();
      return (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="block max-w-[16rem] truncate font-semibold">{title}</span>
          </TooltipTrigger>
          <TooltipContent>{title}</TooltipContent>
        </Tooltip>
      );
    },
  },
  {
    accessorKey: "facility",
    header: "Facility",
    icon: Building2,
    mono: true,
    cell: (c) => c.getValue<string>() || "—",
  },
  {
    accessorKey: "start_time",
    header: "Start (Z)",
    icon: CalendarClock,
    mono: true,
    cell: (c) => <span className="whitespace-nowrap">{formatZuluFull(c.getValue<string>())}</span>,
  },
  {
    accessorKey: "end_time",
    header: "End (Z)",
    mono: true,
    cell: (c) => <span className="whitespace-nowrap text-ink-2">{formatZuluFull(c.getValue<string>())}</span>,
  },
  {
    accessorKey: "review_status",
    header: "Status",
    icon: CircleDot,
    cell: (c) => <ReviewPill status={c.getValue<string>() ?? ""} />,
  },
  {
    accessorKey: "recording",
    header: "Recording",
    icon: Radio,
    cell: (c) => <RecordingPill status={c.getValue<string>() ?? ""} />,
  },
  {
    id: "support",
    header: "Support",
    enableSorting: false,
    cell: (c) => {
      const e = c.row.original;
      if (!e.ace_requested && !e.facility_support) return <span className="text-ink-3">—</span>;
      return (
        <div className="flex flex-wrap gap-1">
          {e.ace_requested && <StatusPill tone="neutral">ACE</StatusPill>}
          {e.facility_support && <StatusPill tone="neutral">Facility</StatusPill>}
        </div>
      );
    },
  },
  {
    id: "actions",
    header: () => <span className="sr-only">Actions</span>,
    enableSorting: false,
    align: "right",
    cell: (c) => (
      <div className="flex items-center justify-end gap-1">
        <EditLink event={c.row.original} />
        <ManageButton event={c.row.original} />
      </div>
    ),
  },
];

function EventCard({ event: e }: { event: EventSummary }) {
  return (
    <Card className="flex flex-col overflow-hidden">
      {e.banner_image_url ? (
        <img src={e.banner_image_url} alt="" className="aspect-[16/7] w-full border-b border-line object-cover" />
      ) : (
        <div className="flex aspect-[16/7] w-full items-center justify-center border-b border-line bg-panel-2">
          <CalendarClock className="size-6 text-ink-3" />
        </div>
      )}
      <div className="flex flex-1 flex-col gap-3 p-4">
        <div className="flex flex-col gap-1">
          <div className="flex items-start justify-between gap-2">
            <h3 className="line-clamp-2 font-semibold leading-snug" title={e.title}>
              {e.title}
            </h3>
            {e.facility && <span className="shrink-0 font-mono text-xs text-ink-2">{e.facility}</span>}
          </div>
          <div className="font-mono text-xs text-ink-2">
            {formatZuluFull(e.start_time)} – {formatZuluFull(e.end_time)}
          </div>
        </div>
        <div className="flex flex-wrap gap-1">
          {e.review_status && <ReviewPill status={e.review_status} />}
          {RECORDING_LABEL[e.recording] && <RecordingPill status={e.recording} />}
          {e.ace_requested && <StatusPill tone="neutral">ACE</StatusPill>}
          {e.facility_support && <StatusPill tone="neutral">Facility</StatusPill>}
        </div>
        <div className="mt-auto flex items-center justify-end gap-1">
          <EditLink event={e} />
          <ManageButton event={e} />
        </div>
      </div>
    </Card>
  );
}

export function PlanningEventsPage() {
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const events = useUpcomingEvents();
  const [scope, setScope] = useState<Scope>("upcoming");
  const view = useView();

  const now = Date.now();
  const filtered = useMemo(() => {
    if (!events.data) return undefined;
    if (scope === "all") return events.data;
    return events.data.filter((e) => (scope === "past" ? endMs(e) < now : endMs(e) >= now));
    // `now` intentionally captured once per render — the split is coarse (event granularity).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [events.data, scope]);

  const board = useMemo(
    () => (filtered ? [...filtered].sort((a, b) => a.start_time.localeCompare(b.start_time)) : []),
    [filtered],
  );

  usePageHeader({
    subtitle: SUBTITLE,
    count: canPlan ? (filtered?.length ?? null) : null,
    views: canPlan ? undefined : null,
  });

  if (!canPlan) {
    return <EmptyState icon={Lock}>You don&apos;t have event planning access yet.</EmptyState>;
  }

  const empty = scope === "past" ? "No recent past events." : "No upcoming events on the VATUSA calendar right now.";

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <SegmentedControl aria-label="Event scope" value={scope} onChange={setScope} options={SCOPES} />
      </FilterBar>

      {view === "board" ? (
        <QueryState
          isLoading={events.isLoading}
          isError={events.isError}
          onRetry={() => events.refetch()}
          isEmpty={board.length === 0}
          empty={empty}
          error="Couldn't load events."
        >
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
            {board.map((e) => (
              <EventCard key={e.id} event={e} />
            ))}
          </div>
        </QueryState>
      ) : (
        <DataTable
          label="Events"
          columns={COLUMNS}
          data={filtered ?? []}
          getRowId={(e) => String(e.id)}
          initialSort={[{ id: "start_time", desc: false }]}
          rowCap={25}
          isLoading={events.isLoading}
          isError={events.isError}
          onRetry={() => events.refetch()}
          empty={empty}
        />
      )}
    </div>
  );
}

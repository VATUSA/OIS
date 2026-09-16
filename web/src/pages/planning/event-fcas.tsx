import {useMemo} from "react";
import {Link, useParams} from "@tanstack/react-router";
import {Button, buttonVariants, ConfirmButton, type DataColumn, DataTable, EmptyState, StatusPill, Switch} from "@ois/ui";
import {Activity, Building2, CircleDot, Gauge, Map as MapIcon, Plus, Type} from "lucide-react";

import {FcaMapView} from "@/components/map/FcaMapView";
import {
  useArchiveEventFca,
  useFcas,
  useFcaTrafficMany,
  usePublishEventFca,
  useSetEventFcaAuto,
  type Fca,
} from "@/lib/fca";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {SectionHeader} from "@/pages/planning/section-header";

const STATUS_LABEL: Record<string, string> = { planned: "Planned", published: "Published", archived: "Archived" };

type FcaRow = Fca & { status: string; crossing?: number };

/**
 * Event manager "FCAs" tab: the event's FCAs with their lifecycle. Planned FCAs can auto-publish
 * (30 min before start) or be published now; published ones can be archived; archived stay as history.
 * The full-screen builder is where they're drawn.
 */
export function EventFcasSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const fcas = useFcas(eventId);
  const list = useMemo(() => fcas.data ?? [], [fcas.data]);

  const { mutate: publish } = usePublishEventFca(eventId);
  const { mutate: archive } = useArchiveEventFca(eventId);
  const { mutate: setAuto } = useSetEventFcaAuto(eventId);

  // Crossing counts for the FCAs that are actually metering (planned + published).
  const activeIds = useMemo(() => list.filter((f) => f.event_status !== "archived").map((f) => f.id), [list]);
  const traffic = useFcaTrafficMany(activeIds);
  const counts = traffic.map((t) => t?.data?.length);
  const countsKey = counts.join(",");
  const rows = useMemo<FcaRow[]>(
    () =>
      list.map((f) => {
        const i = activeIds.indexOf(f.id);
        return { ...f, status: f.event_status ?? "planned", crossing: i >= 0 ? counts[i] : undefined };
      }),
    // `counts` is a fresh array each render; its joined key is the stable dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [list, activeIds, countsKey],
  );

  const columns = useMemo<DataColumn<FcaRow>[]>(
    () => [
      {
        accessorKey: "name",
        header: "FCA",
        icon: Type,
        cell: (c) => (
          <span className="flex items-center gap-2 whitespace-nowrap">
            {/* The FCA's colour is user data. */}
            <span className="size-3 shrink-0 rounded-full" style={{ background: c.row.original.color }} />
            <span className="font-mono font-semibold">{c.row.original.name || "Untitled"}</span>
          </span>
        ),
      },
      {
        id: "rule",
        header: "Rule",
        icon: Gauge,
        mono: true,
        enableSorting: false,
        cell: (c) => (c.row.original.mode === "mit" ? `${c.row.original.mit} MIT` : `${c.row.original.rate}/hr`),
      },
      {
        accessorKey: "artcc",
        header: "ARTCC",
        icon: Building2,
        mono: true,
        cell: (c) => c.getValue<string>() || "—",
      },
      {
        accessorKey: "crossing",
        header: "Crossing",
        icon: Activity,
        mono: true,
        align: "right",
        cell: (c) => (c.row.original.status !== "archived" && c.row.original.crossing != null ? c.row.original.crossing : "—"),
      },
      {
        accessorKey: "status",
        header: "Status",
        icon: CircleDot,
        cell: (c) => {
          const s = c.getValue<string>();
          return STATUS_LABEL[s] ? <StatusPill tone={toneOf("eventFca", s)}>{STATUS_LABEL[s]}</StatusPill> : null;
        },
      },
      {
        id: "actions",
        header: () => <span className="sr-only">Actions</span>,
        enableSorting: false,
        align: "right",
        cell: (c) => {
          const fca = c.row.original;
          if (!canEdit) return null;
          if (fca.status === "planned") {
            return (
              <div className="flex items-center justify-end gap-2">
                <label
                  className="flex items-center gap-1.5 text-xs text-ink-2"
                  title="Automatically publish this FCA 30 minutes before the event starts"
                >
                  <Switch
                    checked={fca.auto_publish}
                    onCheckedChange={(v) => setAuto({ fcaId: fca.id, auto: v })}
                    className="scale-[0.68]"
                  />
                  auto
                </label>
                <Button size="sm" onClick={() => publish(fca.id)}>
                  Publish
                </Button>
                <ConfirmButton
                  size="sm"
                  variant="ghost"
                  onConfirm={() => archive(fca.id)}
                  warn={`Cancel the ${fca.name || "untitled"} FCA?`}
                >
                  Cancel
                </ConfirmButton>
              </div>
            );
          }
          if (fca.status === "published") {
            return (
              <ConfirmButton
                size="sm"
                variant="outline"
                onConfirm={() => archive(fca.id)}
                warn={`Archive the ${fca.name || "untitled"} FCA? It comes off every live map.`}
              >
                Archive
              </ConfirmButton>
            );
          }
          return null;
        },
      },
    ],
    [canEdit, publish, archive, setAuto],
  );

  const builderLink = (
    <Link
      to="/admin/planning/events/$eventId/fcas"
      params={{ eventId: String(eventId) }}
      className={buttonVariants({ size: "sm" })}
    >
      <MapIcon className="size-3.5" />
      {canEdit ? "Open FCA builder" : "Open map"}
    </Link>
  );

  return (
    <section className="flex flex-col gap-4">
      <SectionHeader
        title="FCAs"
        description="FCAs planned for this event. They stay off every live map until published (manually, or automatically 30 min before start), and are archived when the event ends."
        actions={builderLink}
      />

      {fcas.data && rows.length === 0 ? (
        <EmptyState
          className="rounded-md border border-line"
          title="No FCAs planned for this event yet."
          action={
            canEdit && (
              <Link
                to="/admin/planning/events/$eventId/fcas"
                params={{ eventId: String(eventId) }}
                className={buttonVariants({ variant: "outline", size: "sm" })}
              >
                <Plus className="size-3.5" />
                Draw the first FCA
              </Link>
            )
          }
        />
      ) : (
        <DataTable
          label="Event FCAs"
          columns={columns}
          data={rows}
          getRowId={(f) => f.id}
          rowCap={25}
          isLoading={fcas.isLoading}
          isError={fcas.isError}
          onRetry={() => fcas.refetch()}
        />
      )}
    </section>
  );
}

/** Full-screen event FCA builder (`/admin/planning/events/$eventId/fcas`) — the shared FcaMapView scoped to
 *  this event, so drawing/editing here creates event-only FCAs. */
export function EventFcaBuilderPage() {
  const { eventId } = useParams({ from: "/admin/planning/events/$eventId/fcas" });
  const id = Number(eventId);
  if (!Number.isFinite(id)) {
    return (
      <div className="flex h-full items-center justify-center">
        <Link to="/admin/planning/events" className={buttonVariants({ variant: "outline", size: "sm" })}>
          Back to events
        </Link>
      </div>
    );
  }
  return <FcaMapView eventId={id} persistKey={`event-fca-${id}`} />;
}

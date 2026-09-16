import {useMemo} from "react";
import {Link, useParams} from "@tanstack/react-router";
import {Badge, Button, buttonVariants, ConfirmButton, Switch} from "@ois/ui";
import {Map, Plus} from "lucide-react";

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

const STATUS: Record<string, { label: string; variant: "secondary" | "success" | "outline" }> = {
  planned: { label: "Planned", variant: "secondary" },
  published: { label: "Published", variant: "success" },
  archived: { label: "Archived", variant: "outline" },
};

function statusBadge(fca: Fca) {
  const s = fca.event_status ? STATUS[fca.event_status] : null;
  return s ? <Badge variant={s.variant}>{s.label}</Badge> : null;
}

/**
 * Event manager "FCAs" tab: the event's FCAs with their lifecycle. Planned FCAs can auto-publish
 * (30 min before start) or be published now; published ones can be archived; archived stay as history.
 * The full-screen builder is where they're drawn.
 */
export function EventFcasSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const fcas = useFcas(eventId);
  const rows = fcas.data ?? [];

  const publish = usePublishEventFca(eventId);
  const archive = useArchiveEventFca(eventId);
  const setAuto = useSetEventFcaAuto(eventId);

  // Crossing counts for the FCAs that are actually metering (planned + published).
  const activeIds = useMemo(
    () => rows.filter((f) => f.event_status !== "archived").map((f) => f.id),
    [rows],
  );
  const traffic = useFcaTrafficMany(activeIds);
  const countFor = (id: string) => {
    const i = activeIds.indexOf(id);
    return i >= 0 ? traffic[i]?.data?.length : undefined;
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="max-w-2xl text-sm text-muted-foreground">
          FCAs planned for this event. They stay off every live map until published (manually, or
          automatically 30 min before start), and are archived when the event ends.
        </p>
        <Link
          to="/admin/planning/events/$eventId/fcas"
          params={{ eventId: String(eventId) }}
          className={buttonVariants({ size: "sm" })}
        >
          <Map className="size-3.5" />
          {canEdit ? "Open FCA builder" : "Open map"}
        </Link>
      </div>

      {!fcas.data ? (
        <p className="py-8 text-center text-sm text-muted-foreground">Loading…</p>
      ) : rows.length === 0 ? (
        <div className="rounded-md border border-dashed py-10 text-center">
          <p className="text-sm text-muted-foreground">No FCAs planned for this event yet.</p>
          {canEdit && (
            <Link
              to="/admin/planning/events/$eventId/fcas"
              params={{ eventId: String(eventId) }}
              className={buttonVariants({ variant: "outline", size: "sm" }) + " mt-3"}
            >
              <Plus className="size-3.5" />
              Draw the first FCA
            </Link>
          )}
        </div>
      ) : (
        <ul className="divide-y rounded-md border">
          {rows.map((fca) => {
            const status = fca.event_status ?? "planned";
            const count = countFor(fca.id);
            return (
              <li key={fca.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2 text-sm">
                <span className="size-3 shrink-0 rounded-full" style={{ background: fca.color }} />
                <span className="font-mono font-medium">{fca.name || "Untitled"}</span>
                <Badge variant="secondary">
                  {fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`}
                </Badge>
                {fca.artcc && <span className="text-xs text-muted-foreground">{fca.artcc}</span>}
                {status !== "archived" && count != null && (
                  <span className="text-xs text-muted-foreground">{count} crossing</span>
                )}
                <div className="ml-auto flex items-center gap-2">
                  {statusBadge(fca)}
                  {canEdit && status === "planned" && (
                    <>
                      <label
                        className="flex items-center gap-1.5 text-xs text-muted-foreground"
                        title="Automatically publish this FCA 30 minutes before the event starts"
                      >
                        <Switch
                          checked={fca.auto_publish}
                          onCheckedChange={(v) => setAuto.mutate({ fcaId: fca.id, auto: v })}
                          className="scale-[0.68]"
                        />
                        auto
                      </label>
                      <Button size="sm" onClick={() => publish.mutate(fca.id)}>
                        Publish
                      </Button>
                      <ConfirmButton
                        size="sm"
                        variant="ghost"
                        onConfirm={() => archive.mutate(fca.id)}
                        warn={`Cancel the ${fca.name || "untitled"} FCA?`}
                      >
                        Cancel
                      </ConfirmButton>
                    </>
                  )}
                  {canEdit && status === "published" && (
                    <ConfirmButton
                      size="sm"
                      variant="outline"
                      onConfirm={() => archive.mutate(fca.id)}
                      warn={`Archive the ${fca.name || "untitled"} FCA? It comes off every live map.`}
                    >
                      Archive
                    </ConfirmButton>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
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

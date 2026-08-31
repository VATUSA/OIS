import {Link, useParams} from "@tanstack/react-router";
import {Badge, buttonVariants} from "@ois/ui";
import {Map, Plus} from "lucide-react";

import {FcaMapView} from "@/components/map/FcaMapView";
import {useFcas, type Fca} from "@/lib/fca";
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
 * Event manager "FCAs" tab: the planned/published/archived FCAs for this event, plus a link into the
 * full-screen builder. Publish/archive/auto-publish controls arrive in a later pass; here it's the
 * roster + entry point.
 */
export function EventFcasSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const fcas = useFcas(eventId);
  const rows = fcas.data ?? [];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="max-w-2xl text-sm text-muted-foreground">
          FCAs planned for this event. They stay off every live map until published (manually or 30 min
          before start), and are archived when the event ends.
        </p>
        <Link
          to="/planning/events/$eventId/fcas"
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
              to="/planning/events/$eventId/fcas"
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
          {rows.map((fca) => (
            <li key={fca.id} className="flex items-center gap-3 px-3 py-2 text-sm">
              <span className="size-3 shrink-0 rounded-full" style={{ background: fca.color }} />
              <span className="font-mono font-medium">{fca.name || "Untitled"}</span>
              <Badge variant="secondary">{fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`}</Badge>
              {fca.artcc && <span className="text-xs text-muted-foreground">{fca.artcc}</span>}
              <span className="ml-auto">{statusBadge(fca)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Full-screen event FCA builder (`/planning/events/$eventId/fcas`) — the shared FcaMapView scoped to
 *  this event, so drawing/editing here creates event-only FCAs. */
export function EventFcaBuilderPage() {
  const { eventId } = useParams({ from: "/planning/events/$eventId/fcas" });
  const id = Number(eventId);
  if (!Number.isFinite(id)) {
    return (
      <div className="flex h-[calc(100vh-3.5rem)] items-center justify-center">
        <Link to="/planning/events" className={buttonVariants({ variant: "outline", size: "sm" })}>
          Back to events
        </Link>
      </div>
    );
  }
  return <FcaMapView eventId={id} persistKey={`event-fca-${id}`} />;
}

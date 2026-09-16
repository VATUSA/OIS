import {useMemo, useState} from "react";
import {Button, buttonVariants, Card, EmptyState, Modal, QueryState, StatusPill, Tabs} from "@ois/ui";
import {useParams} from "@tanstack/react-router";
import {
  BarChart3,
  CalendarCheck,
  ExternalLink,
  Gauge,
  Layers,
  Lock,
  Map as MapIcon,
  MessageSquare,
  Radio,
  Users,
  Waypoints,
} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {Markdown} from "@/components/markdown";
import {useMe} from "@/lib/auth";
import {eventBodyText, useDcc, useEvent, usePublishEventDiscord, vatusaEditUrl} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {formatZuluFull} from "@/lib/time";
import {DccSection} from "@/pages/planning/dcc";
import {FacilitySupportSection} from "@/pages/planning/facility-support";
import {AirportRatesSection} from "@/pages/planning/airport-rates";
import {AceSection} from "@/pages/planning/ace";
import {AvailabilitySection} from "@/pages/planning/availability";
import {TmiPackagesSection} from "@/pages/planning/tmi-packages";
import {EventFcasSection} from "@/pages/planning/event-fcas";
import {EventStatsSection} from "@/pages/planning/event-stats";

type TabId = "airports" | "facility" | "tmi" | "fcas" | "ace" | "availability" | "stats";
const TABS = [
  { value: "airports", label: "Airports & rates", icon: Gauge },
  { value: "facility", label: "Facility support", icon: Waypoints },
  { value: "tmi", label: "TMI packages", icon: Layers },
  { value: "fcas", label: "FCAs", icon: MapIcon },
  { value: "ace", label: "ACE", icon: Users },
  { value: "availability", label: "Availability", icon: CalendarCheck },
  { value: "stats", label: "Stats & debrief", icon: BarChart3 },
] as const;

const DCC_LABEL: Record<string, string> = { requested: "Requested", confirmed: "Confirmed" };

/** Header actions: DCC dialog, Debrief jump, Discord thread, Edit on VATUSA. */
function EventActions({
  eventId,
  editUrl,
  onDebrief,
}: {
  eventId: number;
  editUrl: string | null;
  onDebrief: () => void;
}) {
  const [dccOpen, setDccOpen] = useState(false);
  const { data: me } = useMe();
  const dcc = useDcc(eventId);
  const dccStatus = dcc.data?.status;
  const canPostDiscord = hasPermission(me, "events.discord.publish");
  const publishDiscord = usePublishEventDiscord(eventId);

  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setDccOpen(true)}>
        <Radio className="size-3.5" />
        DCC support
        {dccStatus && dccStatus !== "not_needed" && (
          <StatusPill tone={toneOf("dcc", dccStatus)} className="-mr-1.5">
            {DCC_LABEL[dccStatus] ?? dccStatus}
          </StatusPill>
        )}
      </Button>

      <Button variant="outline" size="sm" onClick={onDebrief} title="Stats & post-event debrief">
        <BarChart3 className="size-3.5" />
        Debrief
      </Button>

      {canPostDiscord && (
        <Button
          variant="outline"
          size="sm"
          onClick={() => publishDiscord.mutate()}
          disabled={publishDiscord.isPending}
          title="Create the DCC planning thread in Discord (routed by the host's region)"
        >
          <MessageSquare className="size-3.5" />
          Create DCC thread
        </Button>
      )}

      {editUrl && (
        <a href={editUrl} target="_blank" rel="noreferrer" className={buttonVariants({ size: "sm" })}>
          <ExternalLink className="size-3.5" />
          Edit on VATUSA
        </a>
      )}

      <Modal open={dccOpen} onClose={() => setDccOpen(false)} title="DCC support">
        <DccSection eventId={eventId} bare />
      </Modal>
    </>
  );
}

export function EventPlanningPage() {
  const { eventId } = useParams({ from: "/admin/planning/events/$eventId" });
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const id = Number(eventId);
  const event = useEvent(id);
  const [tab, setTab] = useState<TabId>("airports");

  const e = canPlan ? event.data : undefined;
  const editUrl = e ? vatusaEditUrl(e) : null;
  const actions = useMemo(
    () => (e ? <EventActions eventId={id} editUrl={editUrl} onDebrief={() => setTab("stats")} /> : undefined),
    [e, id, editUrl],
  );
  usePageHeader({
    title: e?.title,
    subtitle: e ? [e.facility, `${formatZuluFull(e.start_time)} – ${formatZuluFull(e.end_time)}`].filter(Boolean).join(" · ") : undefined,
    actions,
  });

  if (!canPlan) return <EmptyState icon={Lock}>You don&apos;t have event planning access yet.</EmptyState>;
  if (!e) {
    return (
      <QueryState
        isLoading={event.isLoading}
        isError={event.isError || !event.isLoading}
        loading="Loading event…"
        error="That event isn't on the calendar (it may have ended or been removed)."
      />
    );
  }

  const blurb = eventBodyText(e.body);

  return (
    <div className="flex flex-col gap-6">
      {(e.banner_image_url || blurb || e.review_status) && (
        <Card className="flex flex-col overflow-hidden md:flex-row">
          {e.banner_image_url && (
            <img
              src={e.banner_image_url}
              alt=""
              className="max-h-56 w-full border-b border-line object-cover md:h-48 md:w-80 md:border-b-0 md:border-r"
            />
          )}
          <div className="flex min-w-0 flex-1 flex-col gap-3 p-4">
            {e.review_status && (
              <div className="flex flex-wrap items-center gap-2 text-xs text-ink-3">
                Review
                <StatusPill tone={toneOf("review", e.review_status)}>
                  {e.review_status.charAt(0).toUpperCase() + e.review_status.slice(1)}
                </StatusPill>
              </div>
            )}
            {blurb && <Markdown className="max-h-40 max-w-3xl overflow-y-auto text-sm text-ink-2">{blurb}</Markdown>}
          </div>
        </Card>
      )}

      <div className="flex flex-col gap-4">
        <Tabs value={tab} onChange={setTab} items={TABS} className="border-b border-line pb-2" />

        {tab === "airports" && <AirportRatesSection eventId={id} eventStart={e.start_time} />}
        {tab === "facility" && <FacilitySupportSection eventId={id} eventStart={e.start_time} />}
        {tab === "tmi" && <TmiPackagesSection eventId={id} />}
        {tab === "fcas" && <EventFcasSection eventId={id} />}
        {tab === "ace" && <AceSection eventId={id} eventStart={e.start_time} eventEnd={e.end_time} />}
        {tab === "availability" && <AvailabilitySection eventId={id} />}
        {tab === "stats" && <EventStatsSection eventId={id} />}
      </div>
    </div>
  );
}

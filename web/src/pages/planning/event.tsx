import {useState} from "react";
import {Badge, Button, buttonVariants, Card, CardContent} from "@ois/ui";
import {Link, useParams} from "@tanstack/react-router";
import {ArrowLeft, BarChart3, CalendarClock, ExternalLink, MessageSquare, Radio, Users} from "lucide-react";

import {Modal} from "@/components/modal";
import {useMe} from "@/lib/auth";
import {eventBodyText, useDcc, useEvent, usePublishEventDiscord, useStaffing, vatusaEditUrl} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZuluFull} from "@/lib/time";
import {DccSection} from "@/pages/planning/dcc";
import {FacilitySupportSection} from "@/pages/planning/facility-support";
import {AirportRatesSection} from "@/pages/planning/airport-rates";
import {AceSection} from "@/pages/planning/ace";
import {TmiPackagesSection} from "@/pages/planning/tmi-packages";
import {EventStatsSection} from "@/pages/planning/event-stats";

type TabId = "airports" | "facility" | "tmi" | "stats";
const TABS: { id: TabId; label: string }[] = [
  { id: "airports", label: "Airports & rates" },
  { id: "facility", label: "Facility support" },
  { id: "tmi", label: "TMI packages" },
  { id: "stats", label: "Stats & debrief" },
];

function dccVariant(status: string): "secondary" | "success" | "outline" {
  if (status === "confirmed") return "success";
  if (status === "requested") return "secondary";
  return "outline";
}

/** Top action bar: Edit-on-VATUSA link + DCC / ACE dialog buttons + a Debrief placeholder. */
function ActionBar({
  eventId,
  editUrl,
  onDebrief,
}: {
  eventId: number;
  editUrl: string | null;
  onDebrief: () => void;
}) {
  const [dialog, setDialog] = useState<null | "dcc" | "ace">(null);
  const { data: me } = useMe();
  const dcc = useDcc(eventId);
  const staffing = useStaffing(eventId);
  const openAce = (staffing.data ?? []).filter((s) => s.status === "open").length;
  const dccStatus = dcc.data?.status;
  const canPostDiscord = hasPermission(me, "events.discord.publish");
  const publishDiscord = usePublishEventDiscord(eventId);

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="secondary" size="sm" onClick={() => setDialog("dcc")}>
        <Radio className="size-3.5" />
        DCC support
        {dccStatus && dccStatus !== "not_needed" && (
          <Badge variant={dccVariant(dccStatus)} className="ml-1">
            {dccStatus}
          </Badge>
        )}
      </Button>

      <Button variant="secondary" size="sm" onClick={() => setDialog("ace")}>
        <Users className="size-3.5" />
        ACE requests
        {openAce > 0 && (
          <Badge variant="secondary" className="ml-1">
            {openAce} open
          </Badge>
        )}
      </Button>

      <Button variant="secondary" size="sm" onClick={onDebrief} title="Stats & post-event debrief">
        <BarChart3 className="size-3.5" />
        Debrief
      </Button>

      {canPostDiscord && (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => publishDiscord.mutate()}
          disabled={publishDiscord.isPending}
          title="Post a coordination thread to Discord"
        >
          <MessageSquare className="size-3.5" />
          Post to Discord
        </Button>
      )}

      {editUrl && (
        <a
          href={editUrl}
          target="_blank"
          rel="noreferrer"
          className={buttonVariants({ variant: "outline", size: "sm" }) + " ml-auto"}
        >
          <ExternalLink className="size-3.5" />
          Edit on VATUSA
        </a>
      )}

      <Modal open={dialog === "dcc"} onClose={() => setDialog(null)} title="DCC support">
        <DccSection eventId={eventId} bare />
      </Modal>
      <Modal open={dialog === "ace"} onClose={() => setDialog(null)} title="ACE requests" size="lg">
        <AceSection eventId={eventId} bare />
      </Modal>
    </div>
  );
}

export function EventPlanningPage() {
  const { eventId } = useParams({ from: "/planning/events/$eventId" });
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const id = Number(eventId);
  const event = useEvent(id);
  const [tab, setTab] = useState<TabId>("airports");

  const backLink = (
    <Link
      to="/planning/events"
      className="flex w-fit items-center gap-1 text-sm text-muted-foreground transition-colors hover:text-foreground"
    >
      <ArrowLeft className="size-4" /> All events
    </Link>
  );

  const shell = (msg: string) => (
    <div className="flex flex-col gap-6">
      {backLink}
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">{msg}</CardContent>
      </Card>
    </div>
  );

  if (!canPlan) return shell("You don't have event planning access yet.");
  if (event.isError || (!event.isLoading && !event.data))
    return shell("That event isn't on the calendar (it may have ended or been removed).");
  if (!event.data) return shell("Loading event…");

  const e = event.data;
  const blurb = eventBodyText(e.body);

  return (
    <div className="flex flex-col gap-6">
      {backLink}

      {/* Event header */}
      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="flex flex-col gap-1">
              <h1 className="text-2xl font-semibold tracking-tight">{e.title}</h1>
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
                {e.facility && <span className="font-mono font-medium text-foreground">{e.facility}</span>}
                <span className="flex items-center gap-1.5">
                  <CalendarClock className="size-3.5" />
                  {formatZuluFull(e.start_time)} – {formatZuluFull(e.end_time)}
                </span>
              </div>
            </div>
            {e.review_status && (
              <Badge variant={e.review_status === "approved" ? "success" : "secondary"}>
                {e.review_status}
              </Badge>
            )}
          </div>

          {e.banner_image_url && (
            <img src={e.banner_image_url} alt="" className="max-h-56 w-full rounded-md object-cover" />
          )}

          {blurb && (
            <p className="max-w-3xl whitespace-pre-line text-sm text-muted-foreground">{blurb}</p>
          )}
        </CardContent>
      </Card>

      {/* Action bar */}
      <ActionBar eventId={id} editUrl={vatusaEditUrl(e)} onDebrief={() => setTab("stats")} />

      {/* Tabbed planning area */}
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap gap-1 border-b">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              className={
                "-mb-px border-b-2 px-3 py-2 text-sm transition-colors " +
                (tab === t.id
                  ? "border-primary font-medium text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground")
              }
            >
              {t.label}
            </button>
          ))}
        </div>

        {tab === "airports" && (
          <AirportRatesSection eventId={id} eventStart={e.start_time} />
        )}
        {tab === "facility" && <FacilitySupportSection eventId={id} />}
        {tab === "tmi" && <TmiPackagesSection eventId={id} />}
        {tab === "stats" && <EventStatsSection eventId={id} />}
      </div>
    </div>
  );
}

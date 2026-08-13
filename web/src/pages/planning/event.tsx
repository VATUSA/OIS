import {Badge, Card, CardContent} from "@ois/ui";
import {Link, useParams} from "@tanstack/react-router";
import {ArrowLeft, CalendarClock, Gauge, Layers, type LucideIcon, Users, Waypoints,} from "lucide-react";

import {useMe} from "@/lib/auth";
import {eventBodyText, useEvent} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZulu} from "@/lib/time";
import {DccSection} from "@/pages/planning/dcc";

type Module = {
  icon: LucideIcon;
  title: string;
  description: string;
};

const MODULES: Module[] = [
  {
    icon: Layers,
    title: "TMI packages",
    description:
      "Draft the programs, restrictions, and ground stops for the event, then activate them live.",
  },
  {
    icon: Waypoints,
    title: "Facility support",
    description: "Mark each facility required, preferred, or not required.",
  },
  {
    icon: Users,
    title: "ACE request",
    description: "Request ACE staffing — positions wanted vs signed up.",
  },
  {
    icon: Gauge,
    title: "Airport rates",
    description: "Set per-airport AAR/ADR that feeds the live rate programs.",
  },
];

function ModuleCard({ mod }: { mod: Module }) {
  return (
    <Card className="h-full">
      <CardContent className="flex h-full flex-col gap-2 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <mod.icon className="size-4" />
          </span>
          <span className="font-semibold">{mod.title}</span>
        </div>
        <p className="text-sm text-muted-foreground">{mod.description}</p>
        <span className="mt-auto pt-2 text-xs font-medium uppercase tracking-wide text-muted-foreground/70">
          Coming soon
        </span>
      </CardContent>
    </Card>
  );
}

export function EventPlanningPage() {
  const { eventId } = useParams({ from: "/planning/events/$eventId" });
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const id = Number(eventId);
  const event = useEvent(id);

  const backLink = (
    <Link
      to="/planning/events"
      className="flex w-fit items-center gap-1 text-sm text-muted-foreground transition-colors hover:text-foreground"
    >
      <ArrowLeft className="size-4" /> All events
    </Link>
  );

  if (!canPlan) {
    return (
      <div className="flex flex-col gap-6">
        {backLink}
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have event planning access yet.
          </CardContent>
        </Card>
      </div>
    );
  }

  if (event.isError || (!event.isLoading && !event.data)) {
    return (
      <div className="flex flex-col gap-6">
        {backLink}
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            That event isn&apos;t on the calendar (it may have ended or been removed).
          </CardContent>
        </Card>
      </div>
    );
  }

  if (!event.data) {
    return (
      <div className="flex flex-col gap-6">
        {backLink}
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            Loading event…
          </CardContent>
        </Card>
      </div>
    );
  }

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
                {e.facility && (
                  <span className="font-mono font-medium text-foreground">
                    {e.facility}
                  </span>
                )}
                <span className="flex items-center gap-1.5">
                  <CalendarClock className="size-3.5" />
                  {formatZulu(e.start_time)} – {formatZulu(e.end_time)}
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
            <img
              src={e.banner_image_url}
              alt=""
              className="max-h-56 w-full rounded-md object-cover"
            />
          )}

          {blurb && (
            <p className="max-w-3xl whitespace-pre-line text-sm text-muted-foreground">
              {blurb}
            </p>
          )}
        </CardContent>
      </Card>

      {/* Planning modules */}
      <div className="flex flex-col gap-4">
        <h2 className="text-sm font-medium uppercase tracking-wide text-muted-foreground">
          Planning
        </h2>
        <DccSection eventId={id} />
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {MODULES.map((mod) => (
            <ModuleCard key={mod.title} mod={mod} />
          ))}
        </div>
      </div>
    </div>
  );
}

import {Badge, Card, CardContent} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {CalendarClock} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type EventSummary, useUpcomingEvents} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZulu} from "@/lib/time";

function reviewVariant(
  status: string,
): "success" | "secondary" | "outline" {
  if (status === "approved") return "success";
  if (status === "rejected" || status === "denied") return "outline";
  return "secondary";
}

function EventCard({ event }: { event: EventSummary }) {
  return (
    <Link
      to="/planning/events/$eventId"
      params={{ eventId: String(event.id) }}
      className="block"
    >
      <Card className="h-full transition-colors hover:border-primary/60">
        <CardContent className="flex h-full flex-col gap-3 pt-6">
          <div className="flex items-start justify-between gap-3">
            <span className="font-semibold leading-snug">{event.title}</span>
            {event.review_status && (
              <Badge variant={reviewVariant(event.review_status)}>
                {event.review_status}
              </Badge>
            )}
          </div>
          <div className="mt-auto flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
            {event.facility && (
              <span className="font-mono font-medium text-foreground">
                {event.facility}
              </span>
            )}
            <span className="flex items-center gap-1.5">
              <CalendarClock className="size-3.5" />
              {formatZulu(event.start_time)} – {formatZulu(event.end_time)}
            </span>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

export function PlanningEventsPage() {
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const events = useUpcomingEvents();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Events</h1>
        <p className="text-muted-foreground">
          Upcoming VATUSA events. Open one to plan its TMI package, DCC support,
          facility staffing, and airport rates.
        </p>
      </div>

      {!canPlan ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have event planning access yet.
          </CardContent>
        </Card>
      ) : events.isError ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            Couldn&apos;t load events.
          </CardContent>
        </Card>
      ) : !events.data ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            Loading events…
          </CardContent>
        </Card>
      ) : events.data.length === 0 ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            No upcoming events on the VATUSA calendar right now.
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {events.data.map((event) => (
            <EventCard key={event.id} event={event} />
          ))}
        </div>
      )}
    </div>
  );
}

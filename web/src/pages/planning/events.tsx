import {Badge, Card, CardContent} from "@ois/ui";
import {useNavigate} from "@tanstack/react-router";

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

function EventRow({ event }: { event: EventSummary }) {
  const navigate = useNavigate();
  const open = () =>
    navigate({
      to: "/planning/events/$eventId",
      params: { eventId: String(event.id) },
    });

  return (
    <tr
      className="cursor-pointer border-t transition-colors hover:bg-accent/50"
      onClick={open}
    >
      <td className="py-2 pr-3 font-medium">{event.title}</td>
      <td className="py-2 pr-3 font-mono text-xs">{event.facility || "—"}</td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(event.start_time)}
      </td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(event.end_time)}
      </td>
      <td className="py-2 text-right">
        {event.review_status && (
          <Badge variant={reviewVariant(event.review_status)}>
            {event.review_status}
          </Badge>
        )}
      </td>
    </tr>
  );
}

export function PlanningEventsPage() {
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const events = useUpcomingEvents();

  const sorted = events.data
    ? [...events.data].sort(
        (a, b) =>
          new Date(a.start_time).getTime() - new Date(b.start_time).getTime(),
      )
    : undefined;

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Events</h1>
        <p className="text-muted-foreground">
          Upcoming VATUSA events. Open one to plan its TMI package, DCC support,
          facility staffing, and airport rates.
        </p>
      </div>

      <Card>
        <CardContent className="pt-6">
          {!canPlan ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              You don&apos;t have event planning access yet.
            </p>
          ) : events.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Couldn&apos;t load events.
            </p>
          ) : !sorted ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Loading…
            </p>
          ) : sorted.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No upcoming events on the VATUSA calendar right now.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Event</th>
                    <th className="pb-2 pr-3 font-medium">Facility</th>
                    <th className="pb-2 pr-3 font-medium">Start</th>
                    <th className="pb-2 pr-3 font-medium">End</th>
                    <th className="pb-2 text-right font-medium">Status</th>
                  </tr>
                </thead>
                <tbody>
                  {sorted.map((event) => (
                    <EventRow key={event.id} event={event} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

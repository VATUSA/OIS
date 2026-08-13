import {Card, CardContent} from "@ois/ui";
import {CalendarClock} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";

export function PlanningTmiPage() {
  const { data: me } = useMe();
  const canPlan =
    hasPermission(me, "tmu.tmi.read") || hasPermission(me, "tmu.tmi.create");

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          Event TMI planning
        </h1>
        <p className="text-muted-foreground">
          Draft and schedule traffic-management initiatives ahead of an event, then
          activate them when it goes live.
        </p>
      </div>

      {!canPlan ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have TMU planning access yet.
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-16 text-center">
            <span className="flex size-11 items-center justify-center rounded-md bg-primary/10 text-primary">
              <CalendarClock className="size-6" />
            </span>
            <p className="text-sm font-medium">Coming soon</p>
            <p className="max-w-md text-sm text-muted-foreground">
              This is where you&apos;ll build an event&apos;s TMIs in advance —
              programs, restrictions, and ground stops staged against a timeline and
              rolled out when the event starts.
            </p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

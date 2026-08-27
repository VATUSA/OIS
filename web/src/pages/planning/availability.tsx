import {Badge, Card, CardContent} from "@ois/ui";
import {CalendarCheck} from "lucide-react";

import {timeAgo} from "@/lib/time";
import {type EventAvailability, useEventAvailability} from "@/lib/availability";

const GROUPS: { status: string; label: string; emoji: string; variant: "success" | "secondary" | "destructive" }[] = [
  { status: "available", label: "Available", emoji: "🟢", variant: "success" },
  { status: "partial", label: "Partial / unsure", emoji: "🟡", variant: "secondary" },
  { status: "unavailable", label: "Unavailable", emoji: "🔴", variant: "destructive" },
];

/** Pretty-print an access-control role (e.g. `DCC_STAFF` → `DCC Staff`). */
function roleLabel(role: string): string {
  return role
    .split("_")
    .map((w) => (w.length <= 3 ? w : w[0] + w.slice(1).toLowerCase()))
    .join(" ");
}

function Person({ p }: { p: EventAvailability }) {
  return (
    <li className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
      <span className="font-medium">{p.display_name}</span>
      <span className="font-mono text-xs text-muted-foreground">{p.cid}</span>
      {p.roles.map((r) => (
        <Badge key={r} variant="outline" className="text-[10px]">
          {roleLabel(r)}
        </Badge>
      ))}
      <span className="ml-auto text-xs text-muted-foreground">{timeAgo(p.updated_at)}</span>
    </li>
  );
}

export function AvailabilitySection({ eventId }: { eventId: number }) {
  const { data, isError } = useEventAvailability(eventId);

  const grouped = (status: string) => (data ?? []).filter((p) => p.status === status);

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <CalendarCheck className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">Availability</span>
            <span className="text-xs text-muted-foreground">
              Who’s reacted on the DCC thread. NTMOs respond for NOM; DCC trainees for shadowing.
            </span>
          </div>
        </div>

        {isError ? (
          <p className="py-6 text-center text-sm text-muted-foreground">Couldn’t load availability.</p>
        ) : !data ? (
          <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
        ) : data.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            No responses yet. Availability appears here once staff press the buttons on the DCC thread.
          </p>
        ) : (
          <div className="grid gap-3 md:grid-cols-3">
            {GROUPS.map((g) => {
              const people = grouped(g.status);
              return (
                <div key={g.status} className="flex flex-col gap-2 rounded-lg border bg-muted/20 p-3">
                  <div className="flex items-center gap-2">
                    <span>{g.emoji}</span>
                    <span className="text-sm font-semibold">{g.label}</span>
                    <Badge variant={people.length > 0 ? g.variant : "outline"} className="ml-auto">
                      {people.length}
                    </Badge>
                  </div>
                  {people.length === 0 ? (
                    <p className="py-2 text-center text-xs text-muted-foreground">—</p>
                  ) : (
                    <ul className="flex flex-col gap-1.5 text-sm">
                      {people.map((p) => (
                        <Person key={p.cid} p={p} />
                      ))}
                    </ul>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

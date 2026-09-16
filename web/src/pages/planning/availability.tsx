import {Card, QueryState, StatusPill} from "@ois/ui";

import {toneOf} from "@/lib/status";
import {timeAgo} from "@/lib/time";
import {type EventAvailability, useEventAvailability} from "@/lib/availability";
import {SectionHeader} from "@/pages/planning/section-header";

const GROUPS: { status: string; label: string }[] = [
  { status: "available", label: "Available" },
  { status: "partial", label: "Partial / unsure" },
  { status: "unavailable", label: "Unavailable" },
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
    <li className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-line-soft px-4 py-2 last:border-b-0">
      <span className="font-semibold">{p.display_name}</span>
      <span className="font-mono text-xs text-ink-3">{p.cid}</span>
      {p.roles.map((r) => (
        <StatusPill key={r} tone="neutral">
          {roleLabel(r)}
        </StatusPill>
      ))}
      <span className="ml-auto text-xs text-ink-3">{timeAgo(p.updated_at)}</span>
    </li>
  );
}

export function AvailabilitySection({ eventId }: { eventId: number }) {
  const { data, isLoading, isError, refetch } = useEventAvailability(eventId);

  const grouped = (status: string) => (data ?? []).filter((p) => p.status === status);

  return (
    <section className="flex flex-col gap-4">
      <SectionHeader
        description="Who’s reacted on the DCC thread. NTMOs respond for NOM; DCC trainees for shadowing."
      />

      <QueryState
        isLoading={isLoading}
        isError={isError}
        onRetry={() => refetch()}
        isEmpty={(data?.length ?? 0) === 0}
        error="Couldn’t load availability."
        empty="No responses yet. Availability appears here once staff press the buttons on the DCC thread."
        className="rounded-md border border-line"
      >
        <div className="grid gap-3 md:grid-cols-3">
          {GROUPS.map((g) => {
            const people = grouped(g.status);
            return (
              <Card key={g.status} className="flex flex-col overflow-hidden">
                <div className="flex items-center gap-2 border-b border-line px-4 py-2.5">
                  <StatusPill tone={toneOf("availability", g.status)} dot>
                    {g.label}
                  </StatusPill>
                  <span className="ml-auto font-mono text-sm text-ink-2">{people.length}</span>
                </div>
                {people.length === 0 ? (
                  <p className="py-4 text-center text-xs text-ink-3">—</p>
                ) : (
                  <ul className="flex flex-col text-sm">
                    {people.map((p) => (
                      <Person key={p.cid} p={p} />
                    ))}
                  </ul>
                )}
              </Card>
            );
          })}
        </div>
      </QueryState>
    </section>
  );
}

import {useState} from "react";
import {Button, Card, ConfirmButton, Input, QueryState, StatusPill, Textarea} from "@ois/ui";
import {LifeBuoy, Plus} from "lucide-react";

import {ZuluDateTime} from "@/components/zulu-datetime";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {hhmmZulu, timeAgo} from "@/lib/time";
import {SectionHeader} from "@/pages/planning/section-header";
import {
  type AceRequest,
  useClaimEventAce,
  useCreateEventAce,
  useDecideEventAce,
  useDeleteEventAce,
  useEventAce,
  useReleaseEventAce,
} from "@/lib/ace";

const toUnix = (iso: string) => Math.floor(new Date(iso).getTime() / 1000);
const toIso = (unixS: number) => new Date(unixS * 1000).toISOString();

function CreateForm({ eventId }: { eventId: number }) {
  const create = useCreateEventAce(eventId);
  const [slots, setSlots] = useState("1");
  const [position, setPosition] = useState("");
  const [details, setDetails] = useState("");

  const submit = () => {
    const n = Math.max(1, Math.min(99, Math.round(Number(slots) || 1)));
    create.mutate(
      {
        slots: n,
        position: position.trim() || undefined,
        details: details.trim(),
      },
      {
        onSuccess: () => {
          setSlots("1");
          setPosition("");
          setDetails("");
        },
      },
    );
  };

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="flex items-center gap-2">
        <LifeBuoy className="size-4 text-brand-ink" />
        <span className="font-semibold">Request ACE support</span>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="flex flex-col gap-1">
          <label className="text-xs font-semibold text-ink-2">Slots</label>
          <Input
            type="number"
            min={1}
            max={99}
            value={slots}
            onChange={(e) => setSlots(e.target.value)}
            className="font-mono"
          />
        </div>
        <div className="flex flex-col gap-1 sm:col-span-2">
          <label className="text-xs font-semibold text-ink-2">Position (optional)</label>
          <Input value={position} onChange={(e) => setPosition(e.target.value)} placeholder="DCA_APP" />
        </div>
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs font-semibold text-ink-2">Details</label>
        <Textarea
          value={details}
          onChange={(e) => setDetails(e.target.value)}
          placeholder="What coverage do you need, and when?"
        />
      </div>
      <div>
        <Button disabled={details.trim().length === 0 || create.isPending} onClick={submit}>
          <Plus className="size-4" /> Submit request
        </Button>
      </div>
    </Card>
  );
}

function ClaimForm({
  eventId,
  req,
  eventStart,
  eventEnd,
}: {
  eventId: number;
  req: string;
  eventStart: string;
  eventEnd: string;
}) {
  const claim = useClaimEventAce(eventId);
  const [notes, setNotes] = useState("");
  const [start, setStart] = useState<number | null>(null);
  const [end, setEnd] = useState<number | null>(null);

  const min = toUnix(eventStart);
  const max = toUnix(eventEnd);

  const submit = () => {
    claim.mutate(
      {
        req,
        body: {
          notes: notes.trim() || null,
          start_time: start != null ? toIso(start) : null,
          end_time: end != null ? toIso(end) : null,
        },
      },
      {
        onSuccess: () => {
          setNotes("");
          setStart(null);
          setEnd(null);
        },
      },
    );
  };

  return (
    <div className="flex flex-col gap-2 border-t border-line-soft pt-3">
      <div className="flex flex-col gap-1">
        <label className="text-xs font-semibold text-ink-2">Notes (optional)</label>
        <Input
          className="h-8"
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          placeholder="e.g. can also cover approach"
        />
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <span className="text-xs font-semibold text-ink-2">Available from (Zulu)</span>
          <ZuluDateTime label="Available from" value={start} min={min} max={max} onChange={setStart} onClear={() => setStart(null)} />
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-xs font-semibold text-ink-2">Available to (Zulu)</span>
          <ZuluDateTime label="Available to" value={end} min={min} max={max} onChange={setEnd} onClear={() => setEnd(null)} />
        </div>
      </div>
      <div>
        <Button size="sm" onClick={submit} disabled={claim.isPending}>
          Claim a slot
        </Button>
      </div>
    </div>
  );
}

function RequestCard({
  eventId,
  r,
  eventStart,
  eventEnd,
  canClaim,
  canDecide,
  myCid,
}: {
  eventId: number;
  r: AceRequest;
  eventStart: string;
  eventEnd: string;
  canClaim: boolean;
  canDecide: boolean;
  myCid: number | undefined;
}) {
  const decide = useDecideEventAce(eventId);
  const release = useReleaseEventAce(eventId);
  const remove = useDeleteEventAce(eventId);

  const filled = r.claims_count >= r.slots;
  const iClaimed = myCid != null && r.claims.some((c) => c.cid === myCid);

  return (
    <Card className="flex flex-col gap-2 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="font-mono font-semibold">{r.position || "—"}</span>
          <StatusPill tone={toneOf("ace", r.status)}>{r.status}</StatusPill>
          <StatusPill tone={filled ? "good" : "neutral"}>
            <span className="font-mono">
              {r.claims_count} / {r.slots}
            </span>
            claimed
          </StatusPill>
        </div>
        <span className="text-xs text-ink-3">{timeAgo(r.created_at)}</span>
      </div>

      <p className="whitespace-pre-wrap text-sm">{r.details}</p>

      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-ink-3">
        <span>
          by {r.requested_by_name ?? "?"}
          {r.requested_by_cid ? <span className="font-mono"> ({r.requested_by_cid})</span> : null}
        </span>
        {r.decided_by_name && <span>closed by {r.decided_by_name}</span>}
      </div>

      {r.claims.length > 0 && (
        <ul className="flex flex-col gap-1 border-t border-line-soft pt-2 text-sm">
          {r.claims.map((c) => (
            <li key={c.cid} className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-semibold">{c.display_name}</span>
              {(c.start_time || c.end_time) && (
                <span className="font-mono text-xs text-ink-3">
                  {hhmmZulu(c.start_time)}–{hhmmZulu(c.end_time)}
                </span>
              )}
              {c.notes && <span className="text-xs text-ink-2">{c.notes}</span>}
            </li>
          ))}
        </ul>
      )}

      {canClaim && r.status === "open" && !iClaimed && !filled && (
        <ClaimForm eventId={eventId} req={r.id} eventStart={eventStart} eventEnd={eventEnd} />
      )}

      {canClaim && iClaimed && (
        <div>
          <Button
            size="sm"
            variant="outline"
            onClick={() => release.mutate(r.id)}
            disabled={release.isPending}
          >
            Release my claim
          </Button>
        </div>
      )}

      {canDecide && (
        <div className="flex flex-wrap gap-1.5 border-t border-line-soft pt-2">
          {r.status === "open" && (
            <>
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Mark this request completed?"
                onConfirm={() => decide.mutate({ req: r.id, outcome: "completed" })}
              >
                Complete
              </ConfirmButton>
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Cancel this request?"
                onConfirm={() => decide.mutate({ req: r.id, outcome: "cancelled" })}
              >
                Cancel
              </ConfirmButton>
            </>
          )}
          <ConfirmButton
            size="sm"
            variant="ghost"
            warn="Delete this request permanently?"
            onConfirm={() => remove.mutate(r.id)}
          >
            Delete
          </ConfirmButton>
        </div>
      )}
    </Card>
  );
}

export function AceSection({
  eventId,
  eventStart,
  eventEnd,
  bare = false,
}: {
  eventId: number;
  eventStart: string;
  eventEnd: string;
  bare?: boolean;
}) {
  const { data: me } = useMe();
  const canCreate = hasPermission(me, "ace.requests.create");
  const canClaim = hasPermission(me, "ace.requests.claim");
  const canDecide = hasPermission(me, "ace.requests.decide");
  const requests = useEventAce(eventId);

  const body = (
    <>
      {canCreate && <CreateForm eventId={eventId} />}

      <QueryState
        isLoading={requests.isLoading}
        isError={!requests.data && requests.isError}
        onRetry={() => requests.refetch()}
        isEmpty={(requests.data?.length ?? 0) === 0}
        error="Couldn’t load the requests."
        empty="No ACE requests yet."
        className="rounded-md border border-line"
      >
        <div className="flex flex-col gap-2">
          {requests.data?.map((r) => (
            <RequestCard
              key={r.id}
              eventId={eventId}
              r={r}
              eventStart={eventStart}
              eventEnd={eventEnd}
              canClaim={canClaim}
              canDecide={canDecide}
              myCid={me?.cid}
            />
          ))}
        </div>
      </QueryState>
    </>
  );

  if (bare) return <div className="flex flex-col gap-4">{body}</div>;

  return (
    <section className="flex flex-col gap-4">
      <SectionHeader
        description="Request live coverage; the ACE team claims slots with their availability."
      />
      {body}
    </section>
  );
}

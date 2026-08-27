import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input, useConfirm} from "@ois/ui";
import {LifeBuoy, Network, Plus, Users} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {hhmmZulu, isFridayUtc, timeAgo} from "@/lib/time";
import {
  type AceRequest,
  useClaimEventAce,
  useCreateEventAce,
  useDecideEventAce,
  useDeleteEventAce,
  useEventAce,
  useGenerateTier1,
  useReleaseEventAce,
} from "@/lib/ace";

/** Local `<input type="datetime-local">` value ↔ ISO UTC string, treating the picker as Zulu. */
const toLocalInput = (iso: string): string => {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}T${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
};
const fromLocalInput = (v: string): string | null => {
  const ms = Date.parse(v + "Z");
  return Number.isFinite(ms) ? new Date(ms).toISOString() : null;
};

function statusVariant(s: string): "secondary" | "success" | "outline" | "destructive" {
  if (s === "completed") return "outline";
  if (s === "cancelled") return "destructive";
  return "secondary";
}

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
    <div className="flex flex-col gap-3 rounded-lg border bg-muted/20 p-4">
      <div className="flex items-center gap-2">
        <LifeBuoy className="size-4 text-primary" />
        <span className="font-semibold">Request ACE support</span>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Slots</label>
          <Input
            type="number"
            min={1}
            max={99}
            value={slots}
            onChange={(e) => setSlots(e.target.value)}
            className="tabular-nums"
          />
        </div>
        <div className="flex flex-col gap-1 sm:col-span-2">
          <label className="text-xs font-medium text-muted-foreground">Position (optional)</label>
          <Input value={position} onChange={(e) => setPosition(e.target.value)} placeholder="DCA_APP" />
        </div>
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs font-medium text-muted-foreground">Details</label>
        <textarea
          className="min-h-20 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          value={details}
          onChange={(e) => setDetails(e.target.value)}
          placeholder="What coverage do you need, and when?"
        />
      </div>
      <div>
        <Button disabled={details.trim().length === 0 || create.isPending} onClick={submit}>
          <Plus className="mr-1 size-4" /> Submit request
        </Button>
      </div>
    </div>
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
  const [start, setStart] = useState("");
  const [end, setEnd] = useState("");

  const minLocal = toLocalInput(eventStart);
  const maxLocal = toLocalInput(eventEnd);

  const submit = () => {
    claim.mutate(
      {
        req,
        body: {
          notes: notes.trim() || null,
          start_time: start ? fromLocalInput(start) : null,
          end_time: end ? fromLocalInput(end) : null,
        },
      },
      {
        onSuccess: () => {
          setNotes("");
          setStart("");
          setEnd("");
        },
      },
    );
  };

  return (
    <div className="flex flex-col gap-2 rounded-md border border-dashed p-3">
      <div className="flex flex-col gap-1">
        <label className="text-xs font-medium text-muted-foreground">Notes (optional)</label>
        <Input
          className="h-8"
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          placeholder="e.g. can also cover approach"
        />
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Available from (Zulu)</label>
          <Input
            className="h-8"
            type="datetime-local"
            min={minLocal}
            max={maxLocal}
            value={start}
            onChange={(e) => setStart(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Available to (Zulu)</label>
          <Input
            className="h-8"
            type="datetime-local"
            min={minLocal}
            max={maxLocal}
            value={end}
            onChange={(e) => setEnd(e.target.value)}
          />
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
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/20 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="font-mono font-semibold">{r.position || "—"}</span>
          <Badge variant={statusVariant(r.status)}>{r.status}</Badge>
          <Badge variant={filled ? "success" : "secondary"}>
            {r.claims_count} / {r.slots} claimed
          </Badge>
        </div>
        <span className="text-xs text-muted-foreground">{timeAgo(r.created_at)}</span>
      </div>

      <p className="whitespace-pre-wrap text-sm">{r.details}</p>

      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          by {r.requested_by_name ?? "?"}
          {r.requested_by_cid ? ` (${r.requested_by_cid})` : ""}
        </span>
        {r.decided_by_name && <span>closed by {r.decided_by_name}</span>}
      </div>

      {r.claims.length > 0 && (
        <ul className="flex flex-col gap-1 border-t pt-2 text-sm">
          {r.claims.map((c) => (
            <li key={c.cid} className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-medium">{c.display_name}</span>
              {(c.start_time || c.end_time) && (
                <span className="font-mono text-xs text-muted-foreground">
                  {hhmmZulu(c.start_time)}–{hhmmZulu(c.end_time)}
                </span>
              )}
              {c.notes && <span className="text-xs text-muted-foreground">{c.notes}</span>}
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
        <div className="flex flex-wrap gap-1.5 border-t pt-2">
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
    </div>
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
  const generateTier1 = useGenerateTier1(eventId);
  const confirm = useConfirm();
  const isFno = isFridayUtc(eventStart);

  const runTier1 = async () => {
    const ok = await confirm({
      title: "Generate Tier-1 requests?",
      description:
        "Opens an ACE support request for each neighbouring ARTCC that doesn’t already have one. This is a Friday Night Ops helper.",
      confirmText: "Generate",
    });
    if (ok) generateTier1.mutate();
  };

  const body = (
    <>
      {canCreate && isFno && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-dashed bg-muted/20 p-3">
          <div className="flex items-center gap-2 text-sm">
            <Network className="size-4 text-primary" />
            <span>
              <span className="font-medium">Friday Night Ops.</span> Fan out support requests to the
              host’s Tier-1 neighbours.
            </span>
          </div>
          <Button size="sm" variant="outline" onClick={runTier1} disabled={generateTier1.isPending}>
            Generate Tier-1 requests
          </Button>
        </div>
      )}

      {canCreate && <CreateForm eventId={eventId} />}

      {requests.isError ? (
        <p className="py-6 text-center text-sm text-muted-foreground">Couldn’t load the requests.</p>
      ) : !requests.data ? (
        <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
      ) : requests.data.length === 0 ? (
        <p className="py-6 text-center text-sm text-muted-foreground">No ACE requests yet.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {requests.data.map((r) => (
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
      )}
    </>
  );

  if (bare) return <div className="flex flex-col gap-4">{body}</div>;

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Users className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">ACE support</span>
            <span className="text-xs text-muted-foreground">
              Request live coverage; the ACE team claims slots with their availability.
            </span>
          </div>
        </div>
        {body}
      </CardContent>
    </Card>
  );
}

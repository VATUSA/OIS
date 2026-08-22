import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {LifeBuoy, Plus, Users} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {formatZuluFull, timeAgo} from "@/lib/time";
import {
  type AceRequest,
  useAceRequests,
  useAceTeam,
  useClaimAceRequest,
  useCreateAceRequest,
  useDecideAceRequest,
  useRemoveAceTeamMember,
  useUpsertAceTeamMember,
} from "@/lib/ace";

function statusVariant(s: string): "secondary" | "success" | "outline" | "destructive" {
  if (s === "claimed") return "success";
  if (s === "completed") return "outline";
  if (s === "cancelled") return "destructive";
  return "secondary";
}

function RequestForm() {
  const create = useCreateAceRequest();
  const [artcc, setArtcc] = useState("");
  const [position, setPosition] = useState("");
  const [when, setWhen] = useState("");
  const [details, setDetails] = useState("");

  const submit = () => {
    create.mutate(
      {
        artcc_id: artcc.trim() || undefined,
        position: position.trim() || undefined,
        requested_for: when ? new Date(when).toISOString() : undefined,
        details: details.trim(),
      },
      {
        onSuccess: () => {
          setArtcc("");
          setPosition("");
          setWhen("");
          setDetails("");
        },
      },
    );
  };

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex items-center gap-2">
          <LifeBuoy className="size-4 text-primary" />
          <span className="font-semibold">Request ACE support</span>
        </div>
        <div className="grid gap-3 sm:grid-cols-3">
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-muted-foreground">ARTCC</label>
            <Input
              value={artcc}
              onChange={(e) => setArtcc(e.target.value.toUpperCase())}
              placeholder="ZDC"
              className="font-mono uppercase"
            />
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-muted-foreground">Position (optional)</label>
            <Input value={position} onChange={(e) => setPosition(e.target.value)} placeholder="DCA_APP" />
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-muted-foreground">Needed for (optional)</label>
            <Input type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} />
          </div>
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Details</label>
          <textarea
            className="min-h-[5rem] w-full rounded-md border bg-background p-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
      </CardContent>
    </Card>
  );
}

function RequestCard({
  r,
  canClaim,
  canDecide,
}: {
  r: AceRequest;
  canClaim: boolean;
  canDecide: boolean;
}) {
  const claim = useClaimAceRequest();
  const decide = useDecideAceRequest();
  const terminal = r.status === "completed" || r.status === "cancelled";

  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/20 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          {r.artcc_id && <span className="font-mono font-semibold">{r.artcc_id}</span>}
          {r.position && <span className="text-sm text-muted-foreground">{r.position}</span>}
          <Badge variant={statusVariant(r.status)}>{r.status}</Badge>
        </div>
        <span className="text-xs text-muted-foreground">{timeAgo(r.created_at)}</span>
      </div>

      <p className="whitespace-pre-wrap text-sm">{r.details}</p>

      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          by {r.requested_by_name ?? "?"}
          {r.requested_by_cid ? ` (${r.requested_by_cid})` : ""}
        </span>
        {r.requested_for && <span>needed {formatZuluFull(r.requested_for)}</span>}
        {r.claimed_by_name && <span>claimed by {r.claimed_by_name}</span>}
        {r.decided_by_name && <span>closed by {r.decided_by_name}</span>}
      </div>

      {!terminal && (canClaim || canDecide) && (
        <div className="flex flex-wrap gap-1.5">
          {canClaim && r.status === "open" && (
            <Button size="sm" onClick={() => claim.mutate(r.id)} disabled={claim.isPending}>
              Claim
            </Button>
          )}
          {canDecide && (
            <>
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Mark this request completed?"
                onConfirm={() => decide.mutate({ id: r.id, outcome: "completed" })}
              >
                Complete
              </ConfirmButton>
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Cancel this request?"
                onConfirm={() => decide.mutate({ id: r.id, outcome: "cancelled" })}
              >
                Cancel
              </ConfirmButton>
            </>
          )}
        </div>
      )}
    </div>
  );
}

const FILTERS: { label: string; value: string | undefined }[] = [
  { label: "Open", value: "open" },
  { label: "Claimed", value: "claimed" },
  { label: "All", value: undefined },
];

function Queue({ canClaim, canDecide }: { canClaim: boolean; canDecide: boolean }) {
  const [filter, setFilter] = useState<string | undefined>("open");
  const requests = useAceRequests(filter);

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="font-semibold">Request queue</span>
          <div className="flex gap-1">
            {FILTERS.map((f) => (
              <Button
                key={f.label}
                size="sm"
                variant={filter === f.value ? "secondary" : "ghost"}
                onClick={() => setFilter(f.value)}
              >
                {f.label}
              </Button>
            ))}
          </div>
        </div>
        {requests.isError ? (
          <p className="py-6 text-center text-sm text-muted-foreground">Couldn’t load the queue.</p>
        ) : !requests.data ? (
          <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
        ) : requests.data.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">No requests here.</p>
        ) : (
          <div className="flex flex-col gap-2">
            {requests.data.map((r) => (
              <RequestCard key={r.id} r={r} canClaim={canClaim} canDecide={canDecide} />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function Team({ canManage }: { canManage: boolean }) {
  const team = useAceTeam();
  const upsert = useUpsertAceTeamMember();
  const remove = useRemoveAceTeamMember();
  const [cid, setCid] = useState("");
  const [role, setRole] = useState("");
  const [artcc, setArtcc] = useState("");

  const add = () => {
    const n = Number(cid.trim());
    if (!Number.isFinite(n) || n <= 0) return;
    upsert.mutate(
      { cid: n, role: role.trim() || undefined, artcc_id: artcc.trim() || undefined, active: true },
      {
        onSuccess: () => {
          setCid("");
          setRole("");
          setArtcc("");
        },
      },
    );
  };

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex items-center gap-2">
          <Users className="size-4 text-muted-foreground" />
          <span className="font-semibold">ACE team</span>
        </div>

        {!team.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : team.data.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">No team members yet.</p>
        ) : (
          <ul className="flex flex-col divide-y">
            {team.data.map((m) => (
              <li key={m.id} className="flex items-center justify-between gap-3 py-2 text-sm">
                <span className="min-w-0">
                  <span className="font-medium">{m.display_name}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    CID {m.cid}
                    {m.role ? ` · ${m.role}` : ""}
                    {m.artcc_id ? ` · ${m.artcc_id}` : ""}
                    {m.active ? "" : " · inactive"}
                  </span>
                </span>
                {canManage && (
                  <ConfirmButton
                    size="sm"
                    variant="ghost"
                    warn={`Remove ${m.display_name} from the ACE team?`}
                    onConfirm={() => remove.mutate(m.cid)}
                  >
                    Remove
                  </ConfirmButton>
                )}
              </li>
            ))}
          </ul>
        )}

        {canManage && (
          <div className="flex flex-wrap items-end gap-2 border-t pt-3">
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">CID</label>
              <Input value={cid} onChange={(e) => setCid(e.target.value)} placeholder="1234567" className="h-8 w-28 font-mono" />
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">Role</label>
              <Input value={role} onChange={(e) => setRole(e.target.value)} placeholder="lead" className="h-8 w-28" />
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">ARTCC</label>
              <Input value={artcc} onChange={(e) => setArtcc(e.target.value.toUpperCase())} placeholder="ZDC" className="h-8 w-24 font-mono uppercase" />
            </div>
            <Button size="sm" variant="outline" onClick={add} disabled={upsert.isPending}>
              Add / update
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function AceSupportPage() {
  const { data: me, isLoading } = useMe();
  const canCreate = hasPermission(me, "ace.requests.create");
  const canRead = hasPermission(me, "ace.requests.read");
  const canClaim = hasPermission(me, "ace.requests.claim");
  const canDecide = hasPermission(me, "ace.requests.decide");
  const canTeamRead = hasPermission(me, "ace.team.read");
  const canTeamUpdate = hasPermission(me, "ace.team.update");

  if (isLoading) {
    return <p className="py-10 text-center text-sm text-muted-foreground">Loading…</p>;
  }
  if (!canCreate && !canRead && !canTeamRead) {
    return (
      <div className="mx-auto w-full max-w-3xl">
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have access to ACE support.
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">ACE support</h1>
        <p className="text-muted-foreground">
          Request live coverage support from the ACE team, and — for the team — work the shared queue.
        </p>
      </div>

      {canCreate && <RequestForm />}
      {canRead && <Queue canClaim={canClaim} canDecide={canDecide} />}
      {canTeamRead && <Team canManage={canTeamUpdate} />}
    </div>
  );
}

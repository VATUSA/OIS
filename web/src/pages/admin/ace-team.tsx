import {useState} from "react";
import {Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {Users} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useAceTeam, useRemoveAceTeamMember, useUpsertAceTeamMember} from "@/lib/ace";

export function AdminAceTeam() {
  const { data: me } = useMe();
  const canManage = hasPermission(me, "ace.team.update");
  const team = useAceTeam(true);
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
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">ACE team</h1>
        <p className="text-muted-foreground">
          The national roster of controllers available to claim ACE support requests on events.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-3 pt-6">
          <div className="flex items-center gap-2">
            <Users className="size-4 text-muted-foreground" />
            <span className="font-semibold">Roster</span>
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
                <Input
                  value={cid}
                  onChange={(e) => setCid(e.target.value)}
                  placeholder="1234567"
                  className="h-8 w-28 font-mono"
                />
              </div>
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">Role</label>
                <Input
                  value={role}
                  onChange={(e) => setRole(e.target.value)}
                  placeholder="lead"
                  className="h-8 w-28"
                />
              </div>
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">ARTCC</label>
                <Input
                  value={artcc}
                  onChange={(e) => setArtcc(e.target.value.toUpperCase())}
                  placeholder="ZDC"
                  className="h-8 w-24 font-mono uppercase"
                />
              </div>
              <Button size="sm" variant="outline" onClick={add} disabled={upsert.isPending}>
                Add / update
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

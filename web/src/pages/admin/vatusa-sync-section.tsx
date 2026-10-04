import {useMemo, useState} from "react";
import {Button, Input, Modal, StatusPill} from "@ois/ui";
import {RefreshCw} from "lucide-react";

import {useUserVatusa, useVatusaResync} from "@/lib/access";

type Change = {group: string; artcc_id?: string | null};

const scopeLabel = (c: Change) => `${c.group} · ${c.artcc_id ?? "national"}`;

/** "Manually managed" pill for the editor header: who took the user off VATUSA role sync, and when (#549). */
export function VatusaDetachedPill({cid}: {cid: number}) {
  const vatusa = useUserVatusa(cid);
  if (!vatusa.data?.detached_at) return null;
  const when = new Date(vatusa.data.detached_at).toLocaleDateString();
  const who = vatusa.data.detached_by ? ` by ${vatusa.data.detached_by}` : "";
  return (
    <StatusPill tone="warn">
      Manually managed — not synced from VATUSA ({when}
      {who})
    </StatusPill>
  );
}

/**
 * The user's VATUSA side in the access editor (#549): their VATUSA roles, so a VATUSA-granted group
 * is explicable, and — while they're hand-managed — a Resync that shows exactly what it will change
 * before it does it.
 */
export function VatusaSyncSection({cid}: {cid: number}) {
  const vatusa = useUserVatusa(cid);
  const resync = useVatusaResync();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");

  const rolesByFacility = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const r of vatusa.data?.profile?.roles ?? []) {
      map.set(r.facility, [...(map.get(r.facility) ?? []), r.role]);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [vatusa.data]);

  if (!vatusa.data) return null;
  const {detached_at, resync_grants, resync_revokes} = vatusa.data;
  const noChange = resync_grants.length === 0 && resync_revokes.length === 0;

  function confirm() {
    resync.mutate(
      {cid, reason: reason.trim()},
      {
        onSuccess: () => {
          setOpen(false);
          setReason("");
        },
      },
    );
  }

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">VATUSA roles</h3>
        {detached_at && (
          <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
            <RefreshCw className="size-4" /> Resync from VATUSA
          </Button>
        )}
      </div>
      {rolesByFacility.length === 0 ? (
        <p className="text-xs text-ink-3">No VATUSA roles on record.</p>
      ) : (
        <div className="flex flex-wrap gap-1">
          {rolesByFacility.flatMap(([facility, roles]) =>
            roles.map((role) => (
              <StatusPill key={`${role}@${facility}`} tone="neutral">
                {role} · {facility}
              </StatusPill>
            )),
          )}
        </div>
      )}
      {detached_at && (
        <p className="text-xs text-ink-3">
          Their access was edited by hand, so VATUSA no longer adds or removes their groups. Name,
          rating and facility still sync.
        </p>
      )}

      <Modal
        open={open}
        onClose={() => setOpen(false)}
        title="Resync from VATUSA"
        description="Put this user back on VATUSA role sync. Groups granted by hand are never removed."
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button onClick={confirm} disabled={!reason.trim() || resync.isPending}>
              Resync
            </Button>
          </div>
        }
      >
        <div className="flex flex-col gap-3 text-sm">
          {noChange ? (
            <p className="text-ink-2">Their groups already match VATUSA — only the sync flag changes.</p>
          ) : (
            <>
              {resync_grants.length > 0 && (
                <div className="flex flex-col gap-1">
                  <span className="font-semibold">Will add</span>
                  <div className="flex flex-wrap gap-1">
                    {resync_grants.map((c) => (
                      <StatusPill key={`+${scopeLabel(c)}`} tone="good">
                        {scopeLabel(c)}
                      </StatusPill>
                    ))}
                  </div>
                </div>
              )}
              {resync_revokes.length > 0 && (
                <div className="flex flex-col gap-1">
                  <span className="font-semibold">Will remove</span>
                  <div className="flex flex-wrap gap-1">
                    {resync_revokes.map((c) => (
                      <StatusPill key={`-${scopeLabel(c)}`} tone="bad">
                        {scopeLabel(c)}
                      </StatusPill>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
          <Input
            aria-label="Reason"
            placeholder="Reason (recorded in the audit log)"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
          />
        </div>
      </Modal>
    </section>
  );
}

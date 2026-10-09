import {useState} from "react";
import {useQueryClient} from "@tanstack/react-query";
import {Button, Input, Modal, QueryState, StatusPill} from "@ois/ui";
import {RotateCcw} from "lucide-react";

import {type AccessResetGrant, useVatusaReset, useVatusaResetPreview} from "@/lib/access";

/** The word the admin types to confirm the reset. */
export const RESET_CONFIRM_WORD = "RESET";

const grantLabel = (g: AccessResetGrant) =>
  `${g.granted ? "" : "deny "}${g.name} · ${g.artcc_id ?? "national"} · ${g.source}`;

/**
 * "Reset all access to VATUSA" (#795), for the server admin only: a dry run first, listing every user
 * whose access would change and the grants each gains and loses; applying it needs a reason and the
 * confirmation word. The server refuses anyone else; hiding it is only so no one else sees it.
 */
export function VatusaResetAction() {
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [confirmWord, setConfirmWord] = useState("");
  const preview = useVatusaResetPreview(open);
  const reset = useVatusaReset();
  const queryClient = useQueryClient();

  function close() {
    // The next opening runs a fresh dry run.
    queryClient.removeQueries({queryKey: ["vatusa-reset-preview"]});
    setOpen(false);
    setReason("");
    setConfirmWord("");
    reset.reset();
  }

  function apply() {
    reset.mutate({reason: reason.trim()}, {onSuccess: close});
  }

  const ready = !!reason.trim() && confirmWord === RESET_CONFIRM_WORD && !!preview.data;

  return (
    <>
      <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
        <RotateCcw className="size-4" /> Reset all access to VATUSA
      </Button>
      <Modal
        open={open}
        onClose={close}
        size="lg"
        title="Reset all access to VATUSA"
        description="Pulls VATUSA fresh, then gives every user exactly their system grants plus what VATUSA justifies. Hand-made groups and permissions are removed, except USER and SERVER_ADMIN, and everyone goes back on VATUSA role sync. This cannot be undone; each change is audited with its before state."
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button onClick={apply} disabled={!ready || reset.isPending}>
              {reset.isPending ? "Resetting…" : "Reset access"}
            </Button>
          </div>
        }
      >
        <div className="flex flex-col gap-3 text-sm">
          {preview.data ? (
            <>
              <p className="text-ink-2">
                Dry run from the VATUSA data the last pull stored: {preview.data.users_reset} of{" "}
                {preview.data.users_checked} users would change. The reset pulls VATUSA again first,
                so it can differ if VATUSA changed since.
              </p>
              <ul className="flex max-h-80 flex-col gap-3 overflow-y-auto">
                {preview.data.users.map((u) => (
                  <li key={`${u.cid ?? u.display_name}`} className="flex flex-col gap-1">
                    <span className="font-semibold">
                      {u.display_name}{" "}
                      <span className="font-mono text-xs text-ink-3">{u.cid ?? "no CID"}</span>
                    </span>
                    <div className="flex flex-wrap gap-1">
                      {u.reattached && <StatusPill tone="warn">Back on VATUSA sync</StatusPill>}
                      {u.removed.map((g) => (
                        <StatusPill key={`-${grantLabel(g)}`} tone="bad">
                          − {grantLabel(g)}
                        </StatusPill>
                      ))}
                      {u.added.map((g) => (
                        <StatusPill key={`+${grantLabel(g)}`} tone="good">
                          + {grantLabel(g)}
                        </StatusPill>
                      ))}
                    </div>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <QueryState
              isLoading={preview.isLoading}
              isError={preview.isError}
              error="Couldn't load the dry run."
            />
          )}
          <Input
            aria-label="Reason"
            placeholder="Reason (recorded on every changed user's audit entry)"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
          />
          <Input
            aria-label="Confirmation"
            placeholder={`Type ${RESET_CONFIRM_WORD} to confirm`}
            value={confirmWord}
            onChange={(e) => setConfirmWord(e.target.value)}
          />
          {reset.isError && <p className="text-danger">{reset.error.message}</p>}
        </div>
      </Modal>
    </>
  );
}

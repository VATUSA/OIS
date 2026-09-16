import {useEffect, useState} from "react";
import {Button, Card, QueryState, SegmentedControl, StatusPill, Textarea} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {useDcc, useUpdateDcc} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {formatZulu} from "@/lib/time";
import {SectionHeader} from "@/pages/planning/section-header";

type Status = "not_needed" | "requested" | "confirmed";

const OPTIONS: { value: Status; label: string }[] = [
  { value: "not_needed", label: "Not needed" },
  { value: "requested", label: "Requested" },
  { value: "confirmed", label: "Confirmed" },
];

function statusLabel(status: string): string {
  return OPTIONS.find((o) => o.value === status)?.label ?? status;
}

export function DccSection({ eventId, bare = false }: { eventId: number; bare?: boolean }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const dcc = useDcc(eventId);
  const save = useUpdateDcc(eventId);

  const [status, setStatus] = useState<Status>("not_needed");
  const [notes, setNotes] = useState("");

  // Sync local edit state from the server whenever fresh data arrives.
  useEffect(() => {
    if (dcc.data) {
      setStatus(dcc.data.status as Status);
      setNotes(dcc.data.notes);
    }
  }, [dcc.data]);

  const dirty = !!dcc.data && (status !== dcc.data.status || notes !== dcc.data.notes);

  const body = !dcc.data ? (
    <QueryState isLoading={!dcc.isError} isError={dcc.isError} onRetry={() => dcc.refetch()} />
  ) : !canEdit ? (
    <div className="flex flex-col gap-2 text-sm">
      <div>
        <StatusPill tone={toneOf("dcc", dcc.data.status)}>{statusLabel(dcc.data.status)}</StatusPill>
      </div>
      <p className="whitespace-pre-line text-ink-2">{dcc.data.notes || "No notes."}</p>
    </div>
  ) : (
    <div className="flex flex-col gap-3">
      <SegmentedControl aria-label="DCC status" value={status} onChange={setStatus} options={OPTIONS} className="self-start" />

      <Textarea
        placeholder="Notes — who's coordinating, what's requested, etc."
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
      />

      <div className="flex items-center gap-3">
        <Button onClick={() => save.mutate({ status, notes })} disabled={!dirty || save.isPending}>
          Save
        </Button>
        {dcc.data.updated_at && (
          <span className="text-xs text-ink-3">
            Last set <span className="font-mono">{formatZulu(dcc.data.updated_at)}</span>
            {dcc.data.updated_by ? ` by ${dcc.data.updated_by}` : ""}
          </span>
        )}
      </div>
    </div>
  );

  if (bare) return <div className="flex flex-col gap-4">{body}</div>;

  return (
    <Card className="flex flex-col gap-4 p-4">
      <SectionHeader
        title="DCC support"
        description="Does this event need national DCC coverage?"
        actions={
          dcc.data && <StatusPill tone={toneOf("dcc", dcc.data.status)}>{statusLabel(dcc.data.status)}</StatusPill>
        }
      />
      {body}
    </Card>
  );
}

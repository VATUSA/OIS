import {useEffect, useState} from "react";
import {Badge, Button, Card, CardContent} from "@ois/ui";
import {Radio} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useDcc, useUpdateDcc} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZulu} from "@/lib/time";

type Status = "not_needed" | "requested" | "confirmed";

const OPTIONS: { value: Status; label: string }[] = [
  { value: "not_needed", label: "Not needed" },
  { value: "requested", label: "Requested" },
  { value: "confirmed", label: "Confirmed" },
];

function statusVariant(status: string): "secondary" | "success" | "outline" {
  if (status === "confirmed") return "success";
  if (status === "requested") return "secondary";
  return "outline";
}

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

  const dirty =
    !!dcc.data && (status !== dcc.data.status || notes !== dcc.data.notes);

  function Header() {
    return (
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Radio className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">DCC support</span>
            <span className="text-xs text-muted-foreground">
              Does this event need national DCC coverage?
            </span>
          </div>
        </div>
        {dcc.data && (
          <Badge variant={statusVariant(dcc.data.status)}>
            {statusLabel(dcc.data.status)}
          </Badge>
        )}
      </div>
    );
  }

  const body = !dcc.data ? (
    <p className="py-2 text-sm text-muted-foreground">Loading…</p>
  ) : !canEdit ? (
    <div className="flex flex-col gap-2 text-sm">
      {dcc.data.notes ? (
        <p className="whitespace-pre-line text-muted-foreground">{dcc.data.notes}</p>
      ) : (
        <p className="text-muted-foreground">No notes.</p>
      )}
    </div>
  ) : (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-1">
        {OPTIONS.map((o) => (
          <Button
            key={o.value}
            type="button"
            size="sm"
            variant={status === o.value ? "default" : "secondary"}
            onClick={() => setStatus(o.value)}
          >
            {o.label}
          </Button>
        ))}
      </div>

      <textarea
        className="min-h-20 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        placeholder="Notes — who's coordinating, what's requested, etc."
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
      />

      <div className="flex items-center gap-3">
        <Button onClick={() => save.mutate({ status, notes })} disabled={!dirty || save.isPending}>
          Save
        </Button>
        {dcc.data.updated_at && (
          <span className="text-xs text-muted-foreground">
            Last set {formatZulu(dcc.data.updated_at)}
            {dcc.data.updated_by ? ` by ${dcc.data.updated_by}` : ""}
          </span>
        )}
      </div>
    </div>
  );

  if (bare) return <div className="flex flex-col gap-4">{body}</div>;

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <Header />
        {body}
      </CardContent>
    </Card>
  );
}

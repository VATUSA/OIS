import {useEffect, useState} from "react";
import {Badge, Button, Card, CardContent, Input} from "@ois/ui";
import {Plus, Waypoints, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {
  type FacilitySupport,
  useFacilitySupport,
  useRemoveFacilitySupport,
  useUpsertFacilitySupport,
} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";

type Level = "required" | "preferred" | "not_required";

const LEVELS: { value: Level; label: string }[] = [
  { value: "required", label: "Required" },
  { value: "preferred", label: "Preferred" },
  { value: "not_required", label: "Not req." },
];

function levelVariant(level: string): "success" | "secondary" | "outline" {
  if (level === "required") return "success";
  if (level === "preferred") return "secondary";
  return "outline";
}

function levelLabel(level: string): string {
  return LEVELS.find((l) => l.value === level)?.label ?? level;
}

function FacilityRow({
  eventId,
  row,
  canEdit,
}: {
  eventId: number;
  row: FacilitySupport;
  canEdit: boolean;
}) {
  const upsert = useUpsertFacilitySupport(eventId);
  const remove = useRemoveFacilitySupport(eventId);
  const [notes, setNotes] = useState(row.notes);

  useEffect(() => setNotes(row.notes), [row.notes]);

  if (!canEdit) {
    return (
      <tr className="border-t">
        <td className="py-2 pr-3 font-mono font-medium">{row.facility}</td>
        <td className="py-2 pr-3">
          <Badge variant={levelVariant(row.level)}>{levelLabel(row.level)}</Badge>
        </td>
        <td className="py-2 text-muted-foreground">{row.notes || "—"}</td>
      </tr>
    );
  }

  return (
    <tr className="border-t">
      <td className="py-2 pr-3 font-mono font-medium">{row.facility}</td>
      <td className="py-2 pr-3">
        <div className="flex flex-wrap gap-1">
          {LEVELS.map((l) => (
            <Button
              key={l.value}
              type="button"
              size="sm"
              variant={row.level === l.value ? "default" : "secondary"}
              onClick={() =>
                upsert.mutate({
                  facility: row.facility,
                  body: { level: l.value, notes },
                })
              }
            >
              {l.label}
            </Button>
          ))}
        </div>
      </td>
      <td className="py-2 pr-3">
        <Input
          className="h-8"
          placeholder="notes"
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={() => {
            if (notes !== row.notes) {
              upsert.mutate({
                facility: row.facility,
                body: { level: row.level, notes },
              });
            }
          }}
        />
      </td>
      <td className="py-2 text-right">
        <button
          type="button"
          title={`Remove ${row.facility}`}
          onClick={() => remove.mutate(row.facility)}
          className="text-muted-foreground transition-colors hover:text-destructive"
        >
          <X className="size-4" />
        </button>
      </td>
    </tr>
  );
}

export function FacilitySupportSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const support = useFacilitySupport(eventId);
  const upsert = useUpsertFacilitySupport(eventId);
  const [query, setQuery] = useState("");

  const rows = support.data ?? [];

  function add() {
    const f = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (f.length >= 2 && f.length <= 8 && !rows.some((r) => r.facility === f)) {
      upsert.mutate({ facility: f, body: { level: "required" } });
    }
    setQuery("");
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Waypoints className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">Facility support</span>
            <span className="text-xs text-muted-foreground">
              Which facilities the event needs, and how badly.
            </span>
          </div>
        </div>

        {canEdit && (
          <div className="flex flex-wrap items-center gap-2">
            <Input
              className="w-32 font-mono uppercase"
              maxLength={8}
              placeholder="ZTL / N90"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && add()}
            />
            <Button onClick={add}>
              <Plus />
              Add facility
            </Button>
          </div>
        )}

        {!support.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No facilities added yet.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Facility</th>
                  <th className="pb-2 pr-3 font-medium">Level</th>
                  <th className="pb-2 pr-3 font-medium">Notes</th>
                  {canEdit && <th className="pb-2" />}
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <FacilityRow
                    key={row.facility}
                    eventId={eventId}
                    row={row}
                    canEdit={canEdit}
                  />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

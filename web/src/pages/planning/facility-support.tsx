import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {Plane, Users, Waypoints, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {ArtccCombobox} from "@/components/artcc-combobox";
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

/** Badges explaining why a facility surfaced in the list. */
function WhyCell({ row }: { row: FacilitySupport }) {
  const parts: React.ReactNode[] = [];
  if (row.is_host)
    parts.push(
      <Badge key="host" variant="default">
        Host
      </Badge>,
    );
  if (row.airports.length > 0)
    parts.push(
      <Badge key="apts" variant="secondary" className="gap-1 font-mono">
        <Plane className="size-3" />
        {row.airports.join(", ")}
      </Badge>,
    );
  if (row.has_staffing)
    parts.push(
      <Badge key="ace" variant="secondary" className="gap-1">
        <Users className="size-3" />
        ACE
      </Badge>,
    );
  if (parts.length === 0)
    parts.push(
      <span key="added" className="text-xs text-muted-foreground">
        added manually
      </span>,
    );
  return <div className="flex flex-wrap items-center gap-1">{parts}</div>;
}

function FacilityRow({
  eventId,
  row,
  name,
}: {
  eventId: number;
  row: FacilitySupport;
  name?: string;
}) {
  const upsert = useUpsertFacilitySupport(eventId);
  const remove = useRemoveFacilitySupport(eventId);
  const [notes, setNotes] = useState(row.notes);

  useEffect(() => setNotes(row.notes), [row.notes]);

  const FacilityCell = (
    <td className="py-2 pr-3 align-top">
      <span className="font-mono font-medium">{row.facility}</span>
      {name && <span className="ml-2 text-xs text-muted-foreground">{name}</span>}
      {!row.stored && (
        <span className="ml-2 text-[10px] uppercase tracking-wide text-muted-foreground">
          suggested
        </span>
      )}
    </td>
  );

  if (!row.editable) {
    return (
      <tr className="border-t">
        {FacilityCell}
        <td className="py-2 pr-3 align-top">
          <WhyCell row={row} />
        </td>
        <td className="py-2 pr-3 align-top">
          <Badge variant={levelVariant(row.level)}>{levelLabel(row.level)}</Badge>
        </td>
        <td className="py-2 align-top text-muted-foreground">{row.notes || "—"}</td>
        <td className="py-2" />
      </tr>
    );
  }

  return (
    <tr className={"border-t align-top" + (row.stored ? "" : " text-muted-foreground")}>
      {FacilityCell}
      <td className="py-2 pr-3">
        <WhyCell row={row} />
      </td>
      <td className="py-2 pr-3">
        <div className="flex flex-wrap gap-1">
          {LEVELS.map((l) => (
            <Button
              key={l.value}
              type="button"
              size="sm"
              variant={row.level === l.value && row.stored ? "default" : "secondary"}
              onClick={() =>
                upsert.mutate({ facility: row.facility, body: { level: l.value, notes } })
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
              upsert.mutate({ facility: row.facility, body: { level: row.level, notes } });
            }
          }}
        />
      </td>
      <td className="py-2 text-right">
        {row.stored && (
          <ConfirmButton
            size="icon"
            title={`Clear ${row.facility}`}
            aria-label={`Clear ${row.facility}`}
            onConfirm={() => remove.mutate(row.facility)}
            warn={`Clear the saved support level for ${row.facility}?`}
          >
            <X className="size-4" />
          </ConfirmButton>
        )}
      </td>
    </tr>
  );
}

export function FacilitySupportSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canAdd = hasPermission(me, "events.support.update");
  const support = useFacilitySupport(eventId);
  const upsert = useUpsertFacilitySupport(eventId);
  const facilities = useFacilities();

  const nameById = useMemo(
    () => new Map((facilities.data ?? []).map((f) => [f.id, f.name])),
    [facilities.data],
  );

  const rows = support.data ?? [];

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
              Auto-derived from the host, configured airports, and ACE requests. Confirm a level or
              adjust; facility staff edit only their own row.
            </span>
          </div>
        </div>

        {canAdd && (
          <ArtccCombobox
            exclude={rows.map((r) => r.facility)}
            onSelect={(id) => upsert.mutate({ facility: id, body: { level: "required" } })}
          />
        )}

        {!support.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No facilities involved yet — add airports or ACE requests, or add one below.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Facility</th>
                  <th className="pb-2 pr-3 font-medium">Involvement</th>
                  <th className="pb-2 pr-3 font-medium">Level</th>
                  <th className="pb-2 pr-3 font-medium">Notes</th>
                  <th className="pb-2" />
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <FacilityRow
                    key={row.facility}
                    eventId={eventId}
                    row={row}
                    name={nameById.get(row.facility)}
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

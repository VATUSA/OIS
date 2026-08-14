import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {Users, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {ArtccCombobox} from "@/components/artcc-combobox";
import {type StaffingRequest, useRemoveStaffing, useStaffing, useUpsertStaffing,} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";

type Status = "open" | "met" | "closed";

const STATUSES: { value: Status; label: string }[] = [
  { value: "open", label: "Open" },
  { value: "met", label: "Met" },
  { value: "closed", label: "Closed" },
];

const clampPos = (n: number) => Math.max(0, Math.min(999, Math.round(n)));

function statusVariant(status: string): "secondary" | "success" | "outline" {
  if (status === "met") return "success";
  if (status === "closed") return "outline";
  return "secondary";
}

function StaffingRow({
  eventId,
  row,
  name,
  canEdit,
}: {
  eventId: number;
  row: StaffingRequest;
  name?: string;
  canEdit: boolean;
}) {
  const upsert = useUpsertStaffing(eventId);
  const remove = useRemoveStaffing(eventId);
  const [req, setReq] = useState(String(row.positions_requested));
  const [fil, setFil] = useState(String(row.positions_filled));
  const [notes, setNotes] = useState(row.notes);

  useEffect(() => {
    setReq(String(row.positions_requested));
    setFil(String(row.positions_filled));
    setNotes(row.notes);
  }, [row.positions_requested, row.positions_filled, row.notes]);

  const FacilityCell = (
    <td className="py-2 pr-3">
      <span className="font-mono font-medium">{row.facility}</span>
      {name && <span className="ml-2 text-xs text-muted-foreground">{name}</span>}
    </td>
  );

  if (!canEdit) {
    return (
      <tr className="border-t">
        {FacilityCell}
        <td className="py-2 pr-3 tabular-nums">
          {row.positions_filled}/{row.positions_requested}
        </td>
        <td className="py-2 pr-3">
          <Badge variant={statusVariant(row.status)}>{row.status}</Badge>
        </td>
        <td className="py-2 text-muted-foreground">{row.notes || "—"}</td>
      </tr>
    );
  }

  const save = (status: Status = row.status as Status) => {
    upsert.mutate({
      facility: row.facility,
      body: {
        positions_requested: clampPos(Number(req) || 0),
        positions_filled: clampPos(Number(fil) || 0),
        status,
        notes,
      },
    });
  };

  const saveFieldsIfChanged = () => {
    const r = clampPos(Number(req) || 0);
    const f = clampPos(Number(fil) || 0);
    if (
      r !== row.positions_requested ||
      f !== row.positions_filled ||
      notes !== row.notes
    ) {
      save();
    }
  };

  return (
    <tr className="border-t">
      {FacilityCell}
      <td className="py-2 pr-3">
        <div className="flex items-center gap-1.5 text-muted-foreground">
          <Input
            className="h-8 w-14 tabular-nums"
            type="number"
            min={0}
            max={999}
            value={fil}
            onChange={(e) => setFil(e.target.value)}
            onBlur={saveFieldsIfChanged}
          />
          <span>/</span>
          <Input
            className="h-8 w-14 tabular-nums"
            type="number"
            min={0}
            max={999}
            value={req}
            onChange={(e) => setReq(e.target.value)}
            onBlur={saveFieldsIfChanged}
          />
        </div>
      </td>
      <td className="py-2 pr-3">
        <div className="flex flex-wrap gap-1">
          {STATUSES.map((s) => (
            <Button
              key={s.value}
              type="button"
              size="sm"
              variant={row.status === s.value ? "default" : "secondary"}
              onClick={() => save(s.value)}
            >
              {s.label}
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
          onBlur={saveFieldsIfChanged}
        />
      </td>
      <td className="py-2 text-right">
        <ConfirmButton
          size="icon"
          title={`Remove ${row.facility}`}
          aria-label={`Remove ${row.facility}`}
          onConfirm={() => remove.mutate(row.facility)}
          warn={`Remove ${row.facility} from the ACE team?`}
        >
          <X className="size-4" />
        </ConfirmButton>
      </td>
    </tr>
  );
}

export function AceSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.staffing_requests.create");
  const staffing = useStaffing(eventId);
  const upsert = useUpsertStaffing(eventId);
  const facilities = useFacilities();

  const nameById = useMemo(
    () => new Map((facilities.data ?? []).map((f) => [f.id, f.name])),
    [facilities.data],
  );

  const rows = staffing.data ?? [];

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Users className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">ACE request</span>
            <span className="text-xs text-muted-foreground">
              Positions each facility is hoping for vs signed up (filled / wanted).
            </span>
          </div>
        </div>

        {canEdit && (
          <ArtccCombobox
            exclude={rows.map((r) => r.facility)}
            placeholder="Request ACE at ARTCC…"
            onSelect={(id) =>
              upsert.mutate({
                facility: id,
                body: {
                  positions_requested: 0,
                  positions_filled: 0,
                  status: "open",
                },
              })
            }
          />
        )}

        {!staffing.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No ACE requests yet.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Facility</th>
                  <th className="pb-2 pr-3 font-medium">Filled / wanted</th>
                  <th className="pb-2 pr-3 font-medium">Status</th>
                  <th className="pb-2 pr-3 font-medium">Notes</th>
                  {canEdit && <th className="pb-2" />}
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <StaffingRow
                    key={row.facility}
                    eventId={eventId}
                    row={row}
                    name={nameById.get(row.facility)}
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

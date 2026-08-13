import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, Input} from "@ois/ui";
import {Waypoints, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
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

type Artcc = { id: string; name: string };

/** Rank an ARTCC against a query; lower = better, -1 = no match. */
function matchRank(q: string, a: Artcc): number {
  const id = a.id.toUpperCase();
  const name = a.name.toUpperCase();
  if (id.startsWith(q)) return 0;
  if (id.includes(q)) return 1;
  if (name.startsWith(q)) return 2;
  if (name.includes(q)) return 3;
  return -1;
}

/** Type-to-filter ARTCC picker. Only offers ARTCCs not already in `exclude`. */
function ArtccCombobox({
  options,
  exclude,
  onSelect,
}: {
  options: Artcc[];
  exclude: string[];
  onSelect: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);

  const available = options.filter((a) => !exclude.includes(a.id));
  const q = query.trim().toUpperCase();
  const matches = (
    q
      ? available
          .map((a) => ({ a, rank: matchRank(q, a) }))
          .filter((x) => x.rank >= 0)
          .sort((x, y) => x.rank - y.rank || x.a.id.localeCompare(y.a.id))
          .map((x) => x.a)
      : available
  ).slice(0, 8);

  const pick = (id: string) => {
    onSelect(id);
    setQuery("");
    setOpen(false);
  };

  return (
    <div className="relative w-72">
      <Input
        className="font-mono uppercase"
        placeholder="Add ARTCC — code or name…"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && matches[0]) {
            e.preventDefault();
            pick(matches[0].id);
          } else if (e.key === "Escape") {
            setOpen(false);
          }
        }}
      />
      {open && matches.length > 0 && (
        <ul className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-md border bg-popover p-1 shadow-md">
          {matches.map((a) => (
            <li key={a.id}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(a.id)}
                className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
              >
                <span className="w-10 shrink-0 font-mono font-medium">{a.id}</span>
                <span className="truncate text-muted-foreground">{a.name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function FacilityRow({
  eventId,
  row,
  name,
  canEdit,
}: {
  eventId: number;
  row: FacilitySupport;
  name?: string;
  canEdit: boolean;
}) {
  const upsert = useUpsertFacilitySupport(eventId);
  const remove = useRemoveFacilitySupport(eventId);
  const [notes, setNotes] = useState(row.notes);

  useEffect(() => setNotes(row.notes), [row.notes]);

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
        <td className="py-2 pr-3">
          <Badge variant={levelVariant(row.level)}>{levelLabel(row.level)}</Badge>
        </td>
        <td className="py-2 text-muted-foreground">{row.notes || "—"}</td>
      </tr>
    );
  }

  return (
    <tr className="border-t">
      {FacilityCell}
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
  const facilities = useFacilities();

  const artccs = useMemo<Artcc[]>(
    () =>
      (facilities.data ?? [])
        .filter((f) => f.active)
        .map((f) => ({ id: f.id, name: f.name })),
    [facilities.data],
  );
  const nameById = useMemo(
    () => new Map(artccs.map((a) => [a.id, a.name])),
    [artccs],
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
              Which ARTCCs the event needs, and how badly.
            </span>
          </div>
        </div>

        {canEdit && (
          <ArtccCombobox
            options={artccs}
            exclude={rows.map((r) => r.facility)}
            onSelect={(id) =>
              upsert.mutate({ facility: id, body: { level: "required" } })
            }
          />
        )}

        {!support.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No ARTCCs added yet.
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

import {type ReactNode, useEffect, useMemo, useRef, useState} from "react";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  FilterBar,
  Input,
  SegmentedControl,
  StatusPill,
  useConfirm,
} from "@ois/ui";
import {Building2, Gauge, Network, NotebookPen, Plane, Users, Waypoints, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {ArtccCombobox} from "@/components/artcc-combobox";
import {
  type FacilitySupport,
  useFacilitySupport,
  useGenerateTier1,
  useRemoveFacilitySupport,
  useUpsertFacilitySupport,
} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {isFridayUtc} from "@/lib/time";
import {SectionHeader} from "@/pages/planning/section-header";

type Level = "required" | "preferred" | "not_required";

const LEVELS: { value: Level; label: string }[] = [
  { value: "required", label: "Required" },
  { value: "preferred", label: "Preferred" },
  { value: "not_required", label: "Not req." },
];

function levelLabel(level: string): string {
  return LEVELS.find((l) => l.value === level)?.label ?? level;
}

/** Notes typed but not yet saved, per facility — the level control sends them too, so picking a level
 * right after typing (before the notes blur-save round-trips) can't overwrite them with stale notes. */
type NoteDrafts = Map<string, string>;

/** Pills explaining why a facility surfaced in the list. */
function WhyCell({ row }: { row: FacilitySupport }) {
  const parts: ReactNode[] = [];
  if (row.is_host)
    parts.push(
      <StatusPill key="host" tone="brand">
        Host
      </StatusPill>,
    );
  if (row.airports.length > 0)
    parts.push(
      <StatusPill key="apts" tone="neutral" className="font-mono">
        <Plane className="size-3" />
        {row.airports.join(", ")}
      </StatusPill>,
    );
  if (row.has_staffing)
    parts.push(
      <StatusPill key="ace" tone="neutral">
        <Users className="size-3" />
        ACE
      </StatusPill>,
    );
  if (parts.length === 0)
    parts.push(
      <span key="added" className="text-xs text-ink-3">
        added manually
      </span>,
    );
  return <div className="flex flex-wrap items-center gap-1">{parts}</div>;
}

function LevelCell({ eventId, row, drafts }: { eventId: number; row: FacilitySupport; drafts: NoteDrafts }) {
  const upsert = useUpsertFacilitySupport(eventId);
  if (!row.editable) {
    return <StatusPill tone={toneOf("support", row.level)}>{levelLabel(row.level)}</StatusPill>;
  }
  // An unsaved suggestion shows no selected level until one is picked.
  return (
    <SegmentedControl<Level | "">
      aria-label={`${row.facility} support level`}
      size="sm"
      value={row.stored ? (row.level as Level) : ""}
      onChange={(level) =>
        level && upsert.mutate({ facility: row.facility, body: { level, notes: drafts.get(row.facility) ?? row.notes } })
      }
      options={LEVELS}
    />
  );
}

function NotesCell({ eventId, row, drafts }: { eventId: number; row: FacilitySupport; drafts: NoteDrafts }) {
  const upsert = useUpsertFacilitySupport(eventId);
  const [notes, setNotes] = useState(row.notes);
  useEffect(() => {
    setNotes(row.notes);
    drafts.delete(row.facility);
  }, [row.notes, row.facility, drafts]);

  if (!row.editable) return <span className="text-ink-2">{row.notes || "—"}</span>;
  return (
    <Input
      aria-label={`${row.facility} notes`}
      className="h-8 min-w-40"
      placeholder="notes"
      value={notes}
      onChange={(e) => {
        setNotes(e.target.value);
        drafts.set(row.facility, e.target.value);
      }}
      onBlur={() => {
        if (notes !== row.notes) {
          upsert.mutate({ facility: row.facility, body: { level: row.level, notes } });
        }
      }}
    />
  );
}

function ClearCell({ eventId, row }: { eventId: number; row: FacilitySupport }) {
  const remove = useRemoveFacilitySupport(eventId);
  if (!row.editable || !row.stored) return null;
  return (
    <ConfirmButton
      size="icon"
      title={`Clear ${row.facility}`}
      aria-label={`Clear ${row.facility}`}
      onConfirm={() => remove.mutate(row.facility)}
      warn={`Clear the saved support level for ${row.facility}?`}
    >
      <X className="size-4" />
    </ConfirmButton>
  );
}

export function FacilitySupportSection({ eventId, eventStart }: { eventId: number; eventStart: string }) {
  const { data: me } = useMe();
  const canAdd = hasPermission(me, "events.support.update");
  const support = useFacilitySupport(eventId);
  const upsert = useUpsertFacilitySupport(eventId);
  const facilities = useFacilities();
  const generateTier1 = useGenerateTier1(eventId);
  const confirm = useConfirm();

  const runTier1 = async () => {
    const ok = await confirm({
      title: "Generate Tier-1 requests?",
      description:
        "Opens an ACE support request for each neighbouring ARTCC that doesn’t already have one. This is a Friday Night Ops helper.",
      confirmText: "Generate",
    });
    if (ok) generateTier1.mutate();
  };

  const nameById = useMemo(() => new Map((facilities.data ?? []).map((f) => [f.id, f.name])), [facilities.data]);

  const rows = support.data ?? [];
  const drafts = useRef<NoteDrafts>(new Map()).current;

  const columns = useMemo<DataColumn<FacilitySupport>[]>(
    () => [
      {
        accessorKey: "facility",
        header: "Facility",
        icon: Building2,
        cell: (c) => {
          const row = c.row.original;
          const name = nameById.get(row.facility);
          return (
            <span className={row.editable && !row.stored ? "whitespace-nowrap text-ink-2" : "whitespace-nowrap"}>
              <span className="font-mono font-semibold">{row.facility}</span>
              {name && <span className="ml-2 text-xs text-ink-3">{name}</span>}
              {!row.stored && (
                <StatusPill tone="neutral" className="ml-2">
                  suggested
                </StatusPill>
              )}
            </span>
          );
        },
      },
      {
        id: "involvement",
        header: "Involvement",
        icon: Waypoints,
        enableSorting: false,
        cell: (c) => <WhyCell row={c.row.original} />,
      },
      {
        accessorKey: "level",
        header: "Level",
        icon: Gauge,
        cell: (c) => <LevelCell eventId={eventId} row={c.row.original} drafts={drafts} />,
      },
      {
        accessorKey: "notes",
        header: "Notes",
        icon: NotebookPen,
        enableSorting: false,
        cell: (c) => <NotesCell eventId={eventId} row={c.row.original} drafts={drafts} />,
      },
      {
        id: "actions",
        header: () => <span className="sr-only">Actions</span>,
        enableSorting: false,
        align: "right",
        cell: (c) => <ClearCell eventId={eventId} row={c.row.original} />,
      },
    ],
    [eventId, nameById],
  );

  return (
    <section className="flex flex-col gap-4">
      <SectionHeader
        description="Auto-derived from the host, configured airports, and ACE requests. Confirm a level or adjust; facility staff edit only their own row."
      />

      {canAdd && isFridayUtc(eventStart) && (
        <Card className="flex flex-wrap items-center justify-between gap-2 p-3">
          <div className="flex items-center gap-2 text-sm">
            <Network className="size-4 text-brand-ink" />
            <span>
              <span className="font-semibold">Friday Night Ops.</span> Fan out support requests to the host’s
              Tier-1 neighbours.
            </span>
          </div>
          <Button size="sm" variant="outline" onClick={runTier1} disabled={generateTier1.isPending}>
            Generate Tier-1 requests
          </Button>
        </Card>
      )}

      {canAdd && (
        <FilterBar>
          <ArtccCombobox
            exclude={rows.map((r) => r.facility)}
            onSelect={(id) => upsert.mutate({ facility: id, body: { level: "required" } })}
          />
        </FilterBar>
      )}

      <DataTable
        label="Facility support"
        columns={columns}
        data={rows}
        getRowId={(r) => r.facility}
        rowCap={25}
        isLoading={support.isLoading}
        isError={!support.data && support.isError}
        onRetry={() => support.refetch()}
        empty="No facilities involved yet — add airports or ACE requests, or add one above."
      />
    </section>
  );
}

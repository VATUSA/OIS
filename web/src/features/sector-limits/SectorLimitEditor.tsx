import * as React from "react";
import {DataTable, type DataColumn, SectorLimitInput, StatusPill, useToast} from "@ois/ui";

import {type SectorLimit, useSectorLimits, useSetSectorLimit} from "./sector-limits";

const TIER: Record<string, string> = {
  low: "Low",
  high: "High",
  ultra_high: "Ultra high",
  approach: "Approach",
};

type Editing = {
  editable: boolean;
  commit: (sectorId: string, limit: number) => void;
  /** Bumped per sector when a write is refused, to remount that sector's input on the stored value. */
  resets: Readonly<Record<string, number>>;
};

const EditingContext = React.createContext<Editing>({ editable: false, commit: () => {}, resets: {} });

function LimitCell({ sector }: { sector: SectorLimit }) {
  const { editable, commit, resets } = React.useContext(EditingContext);
  if (!editable) return <span className="px-1 text-ink-2">{sector.limit}</span>;
  return (
    <SectorLimitInput
      key={resets[sector.sector_id] ?? 0}
      sectorId={sector.sector_id}
      limit={sector.limit}
      onCommit={commit}
    />
  );
}

// Module-level so each cell renderer keeps its identity: `DataTable` renders a cell function as a
// component, and a new one per render would remount every input and drop a draft mid-typing.
const COLUMNS: DataColumn<SectorLimit>[] = [
  { id: "sector", header: "Sector", accessorKey: "sector_id", mono: true, cellClassName: "font-semibold text-ink" },
  { id: "tier", header: "Tier", accessorFn: (s) => TIER[s.tier] ?? s.tier, cellClassName: "text-ink-2" },
  {
    id: "limit",
    header: "Limit",
    accessorKey: "limit",
    align: "right",
    mono: true,
    cell: ({ row }) => <LimitCell sector={row.original} />,
  },
  {
    id: "source",
    header: "Source",
    accessorFn: (s) => (s.overridden ? "Override" : "Default"),
    cell: ({ row }) =>
      row.original.overridden ? <StatusPill>Override</StatusPill> : <span className="text-ink-3">Default</span>,
  },
];

/**
 * One ARTCC's sector occupancy limits (#722), for the Operations page to place (#725). The limit is an
 * input only when the server says the viewer may set this ARTCC's limits; anywhere else it is plain
 * text with no affordance, because a control you cannot use should not hint that you can.
 *
 * The input is `SectorGrid`'s own: it commits a positive whole number that differs from the current
 * limit, and anything else (zero, negative, non-numeric, empty, unchanged, Escape) reverts without a
 * write — so an invalid entry can never clear an override.
 */
export function SectorLimitEditor({ artcc }: { artcc: string }) {
  const id = artcc.trim().toUpperCase();
  const q = useSectorLimits(id);
  const { mutate } = useSetSectorLimit(id);
  const toast = useToast();
  const [resets, setResets] = React.useState<Record<string, number>>({});

  const commit = React.useCallback(
    (sectorId: string, limit: number) =>
      mutate(
        { sectorId, limit },
        {
          // A refused write leaves the limit unchanged, so the input would keep showing the number
          // typed into it; remounting it puts the stored value back.
          onError: () => {
            setResets((r) => ({ ...r, [sectorId]: (r[sectorId] ?? 0) + 1 }));
            toast.error(`Couldn't set ${sectorId}'s limit`, { description: "The limit is unchanged." });
          },
        },
      ),
    [mutate, toast],
  );
  const editing = React.useMemo(
    () => ({ editable: q.data?.editable ?? false, commit, resets }),
    [q.data?.editable, commit, resets],
  );

  return (
    <EditingContext.Provider value={editing}>
      <DataTable
        columns={COLUMNS}
        data={q.data?.sectors ?? []}
        getRowId={(s) => s.sector_id}
        rowCap={Infinity}
        pageSize={100}
        isLoading={q.isLoading}
        isError={q.isError}
        onRetry={() => void q.refetch()}
        empty={`No sector data for ${id}`}
        label={`${id} sector limits`}
      />
    </EditingContext.Provider>
  );
}

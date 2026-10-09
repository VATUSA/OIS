import {useState} from "react";
import {useQueryClient} from "@tanstack/react-query";

import {useSetSectorLimit} from "@/features/sector-limits/sector-limits";

import {MonitorTable, type TableEditing, useNeighbourOpen} from "./MonitorTable";
import {
  type SectorDemand,
  type SectorDemandTable,
  WriteError,
  sectorDemandKey,
  useConsolidateSectors,
  useSectorDemand,
} from "./sector-demand";
import {
  type Consolidation,
  type ConsolidationPatch,
  type DemandTableKind,
  consolidationError,
  consolidationOf,
  sectorLabel,
  withPending,
} from "./view";
import {C, FONT, RETRO_CSS, WEIGHT} from "./vtbfm-palette";

type Query = ReturnType<typeof useSectorDemand>;

/** What a table body says while its facility has nothing to draw, or undefined once it has. */
function stateMessage(artcc: string, q: Query): string | undefined {
  const data = q.data;
  if (!data) return q.isError ? `Couldn't load ${artcc}'s sector demand.` : "Waiting for the first sector-monitor cycle…";
  if (data.status === "pending") return "Waiting for the first sector-monitor cycle…";
  return undefined;
}

/** Every sector a table can name in its menu: its rows, the sectors worked at them, and pending moves onto them. */
function universeOf(table: SectorDemandTable | undefined, pending: Readonly<ConsolidationPatch>): string[] {
  const rows = table?.rows ?? [];
  const here = new Set(rows.flatMap((r) => [r.sector_id, ...r.consolidated]));
  for (const [s, t] of Object.entries(pending)) if (t !== null && here.has(t)) here.add(s);
  return [...here];
}

/**
 * One facility's two tables, enroute then TRACON, drawn the same way whether it is the selected
 * facility or a neighbour. A facility with no sector volumes at all draws one table naming that (#727).
 */
function FacilityTables({
  artcc,
  q,
  consolidation,
  pending = {},
  pendingMaps = {},
  editing,
  neighbourOpen,
}: {
  artcc: string;
  q: Query;
  consolidation: Consolidation;
  pending?: Readonly<ConsolidationPatch>;
  pendingMaps?: Readonly<Record<string, number>>;
  editing?: TableEditing;
  neighbourOpen?: Record<DemandTableKind, { open: boolean; onToggle: () => void }>;
}) {
  const data: SectorDemand | undefined = q.data;
  const message = stateMessage(artcc, q);
  const ready = data?.status === "ready";
  const kinds: DemandTableKind[] = data?.status === "no_sector_data" ? ["enroute"] : ["enroute", "tracon"];
  return (
    <>
      {kinds.map((kind) => (
        <MonitorTable
          key={kind}
          artcc={artcc}
          kind={kind}
          table={ready ? data[kind] : undefined}
          message={data?.status === "no_sector_data" ? `No sector data for ${artcc}` : message}
          binStartsMs={data?.bin_starts_ms ?? []}
          universe={universeOf(ready ? data[kind] : undefined, pending)}
          consolidation={consolidation}
          pendingMaps={pendingMaps}
          editing={editing}
          neighbour={neighbourOpen?.[kind]}
        />
      ))}
    </>
  );
}

/**
 * A neighbour's tables: collapsed until opened, open state remembered per browser per ARTCC and table,
 * and view-only whatever the viewer may edit there — no MAP editor and no menu. The server refuses
 * the write too (403 outside the viewer's scope). Nothing is fetched until one of them is opened.
 */
function NeighbourTables({ artcc }: { artcc: string }) {
  const [enroute, setEnroute] = useNeighbourOpen(artcc, "enroute");
  const [tracon, setTracon] = useNeighbourOpen(artcc, "tracon");
  const q = useSectorDemand(artcc, { enabled: enroute || tracon });
  const rows = q.data?.status === "ready" ? [...q.data.enroute.rows, ...q.data.tracon.rows] : [];
  return (
    <FacilityTables
      artcc={artcc}
      q={q}
      consolidation={consolidationOf(rows)}
      neighbourOpen={{
        enroute: { open: enroute, onToggle: () => setEnroute(!enroute) },
        tracon: { open: tracon, onToggle: () => setTracon(!tracon) },
      }}
    />
  );
}

/** Drops `patch`'s keys from `pending` where they still hold the values `patch` set. */
function without<T>(pending: Readonly<Record<string, T>>, patch: Readonly<Record<string, T>>): Record<string, T> {
  const next = { ...pending };
  for (const [k, v] of Object.entries(patch)) if (next[k] === v) delete next[k];
  return next;
}

/**
 * The selected facility's whole monitor (#794): vTBFM's beige page with its enroute and TRACON tables,
 * then each neighbour's two. The page keys this by facility, so a switch replaces every table, control
 * and open neighbour.
 *
 * MAP and consolidation writes are optimistic, as in vTBFM: the board shows the change at once and
 * keeps it until the refetch after the write settles, when the server's answer takes over, or rolls
 * it back and says why if the write is refused.
 */
export function FacilityDemand({ artcc }: { artcc: string }) {
  const qc = useQueryClient();
  const q = useSectorDemand(artcc);
  const data = q.data;
  const setLimit = useSetSectorLimit(artcc);
  const consolidate = useConsolidateSectors(artcc);
  const [pendingMaps, setPendingMaps] = useState<Readonly<Record<string, number>>>({});
  const [pending, setPending] = useState<Readonly<ConsolidationPatch>>({});
  const [mapError, setMapError] = useState<string | null>(null);
  const [consError, setConsError] = useState<string | null>(null);

  const rows = data?.status === "ready" ? [...data.enroute.rows, ...data.tracon.rows] : [];
  const consolidation = withPending(consolidationOf(rows), pending);
  const refetch = () => qc.invalidateQueries({ queryKey: sectorDemandKey(artcc) });

  const onSetMap = (sector: string, limit: number) => {
    const patch = { [sector]: limit };
    setPendingMaps((p) => ({ ...p, ...patch }));
    setMapError(null);
    // `mutateAsync`, per call: a second `mutate` on the same mutation detaches the first call's
    // callbacks, so a quick second edit would strand the first one's overlay or skip its rollback.
    void setLimit.mutateAsync({ sectorId: sector, limit }).then(
      () => refetch().finally(() => setPendingMaps((p) => without(p, patch))),
      () => {
        setPendingMaps((p) => without(p, patch));
        setMapError(`Could not save MAP for ${sectorLabel(artcc, sector)} — check TMU access / connection.`);
      },
    );
  };

  const onConsolidate = (patch: ConsolidationPatch) => {
    if (Object.keys(patch).length === 0) return;
    // What the write is judged against, for naming the sector in a refusal.
    const before = consolidation;
    const known = new Set(rows.flatMap((r) => [r.sector_id, ...r.consolidated]));
    setPending((p) => ({ ...p, ...patch }));
    setConsError(null);
    // Per call, like the MAP edit: the checklist sends several writes in quick succession.
    void consolidate.mutateAsync(patch).then(
      () => refetch().finally(() => setPending((p) => without(p, patch))),
      (e: unknown) => {
        setPending((p) => without(p, patch));
        const status = e instanceof WriteError ? e.status : undefined;
        setConsError(consolidationError(status, artcc, patch, before, known));
      },
    );
  };

  const editing: TableEditing = {
    onSetMap: data?.limits_editable ? onSetMap : undefined,
    onConsolidate: data?.consolidations_editable ? onConsolidate : undefined,
  };

  return (
    // `colorScheme: light`: the OIS root is dark, which would draw vTBFM's checkbox and select dark.
    <div style={{ background: C.beige, colorScheme: "light", display: "flex", flexDirection: "column", gap: 8, padding: "8px 0", minWidth: 0 }}>
      <style>{RETRO_CSS}</style>
      {(mapError || consError) && (
        <div role="alert" style={{ display: "flex", gap: 12, padding: "0 8px", fontFamily: FONT, fontSize: 12, fontWeight: WEIGHT, color: C.error }}>
          {mapError && <span>{mapError}</span>}
          {consError && <span>{consError}</span>}
        </div>
      )}
      <FacilityTables
        artcc={artcc}
        q={q}
        consolidation={consolidation}
        pending={pending}
        pendingMaps={pendingMaps}
        editing={editing}
      />
      {(data?.neighbours ?? []).map((n) => (
        <NeighbourTables key={n} artcc={n} />
      ))}
    </div>
  );
}

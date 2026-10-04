import {useEffect, useMemo, useState} from "react";
import {
  Button,
  type DataColumn,
  DataTable,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  FilterBar,
  QueryState,
  Select,
  cn,
  useLocalStorage,
} from "@ois/ui";
import {ArrowDown, ArrowUp, ChevronDown, ChevronRight, Gauge, Hash, Radio} from "lucide-react";

import {SectorMapCell} from "@/components/sector-map-cell";
import {usePageHeader} from "@/components/shell/page-meta";
import {applyOrder} from "@/components/map/lib/order-storage";
import {useFacilities} from "@/lib/admin";
import {
  ALERT_WINDOWS,
  DEFAULT_ALERT_WINDOW,
  DEFAULT_TIME_RANGE,
  type MonitorRow,
  TIME_RANGES,
  alertedWithin,
  defaultOrder,
  moveRow,
  sliceBins,
  useConsolidate,
  useConsolidateAll,
  useMonitorNeighbours,
  useMonitorTable,
  useReleaseSector,
} from "@/lib/monitor";
import {alertTextClass} from "@/lib/monitor-alert";
import {hhmmZulu} from "@/lib/time";

/** The open row menu: which sector, and where the pointer was. */
type RowMenu = {sector: MonitorRow; x: number; y: number};

/**
 * One ARTCC's Monitor table (#601), from `GET /api/v1/flow/monitor/{artcc}` (#701). Everything a viewer
 * adjusts — Time Range, the alert filter, row order — is per browser and per centre; none of it
 * refetches. MAP edits and the row menu work only where the server says this caller may edit, and
 * never on a neighbour's table (`viewOnly`).
 */
export function MonitorTable({artcc, viewOnly = false}: {artcc: string; viewOnly?: boolean}) {
  const monitor = useMonitorTable(artcc);
  const [hours, setHours] = useLocalStorage<number>(`ois.monitor.${artcc}.range`, DEFAULT_TIME_RANGE);
  const [alertHours, setAlertHours] = useLocalStorage<number>(`ois.monitor.${artcc}.alert`, DEFAULT_ALERT_WINDOW);
  const [order, setOrder] = useLocalStorage<string[]>(`ois.monitor.${artcc}.order`, []);
  const [menu, setMenu] = useState<RowMenu | null>(null);
  const consolidate = useConsolidate(artcc);
  const release = useReleaseSector(artcc);
  const consolidateAll = useConsolidateAll(artcc);

  // A neighbour's table is theirs to change, never yours, whatever the server would allow (#712).
  const editable = !viewOnly && (monitor.data?.editable ?? false);
  const allRows = useMemo(() => monitor.data?.rows ?? [], [monitor.data]);
  const ordered = useMemo(() => {
    const byId = new Map(allRows.map((r) => [r.sector_id, r]));
    const ids = applyOrder(defaultOrder([...byId.keys()]), order);
    return ids.map((id) => byId.get(id)).filter((r): r is MonitorRow => r != null);
  }, [allRows, order]);
  const now = monitor.data ? Date.parse(monitor.data.as_of) : Date.now();
  const visible = ordered.filter((r) => alertedWithin(r, alertHours, now));
  const visibleIds = new Set(visible.map((r) => r.sector_id));
  const shownBins = sliceBins(ordered[0]?.bins ?? [], hours);

  const move = (id: string, dir: 1 | -1) =>
    setOrder(moveRow(ordered.map((r) => r.sector_id), id, dir, visibleIds));

  const columns: DataColumn<MonitorRow>[] = [
    {
      id: "sector",
      header: "Sector",
      icon: Hash,
      cell: (c) => {
        const r = c.row.original;
        return (
          <span className="inline-flex items-center gap-1.5 font-mono">
            {r.sector_id}
            {r.consolidated.length > 0 && "+"}
            {r.name && <span className="font-sans text-xs text-ink-3">{r.name}</span>}
            {r.staffed && <Radio aria-label="Staffed" className="size-3.5 text-ink-2" />}
          </span>
        );
      },
    },
    {
      id: "map",
      header: "MAP",
      icon: Gauge,
      cell: (c) => (
        <SectorMapCell artcc={artcc} sectorId={c.row.original.sector_id} map={c.row.original.map} editable={editable} />
      ),
    },
    ...shownBins.map(
      (_, i): DataColumn<MonitorRow> => ({
        id: `bin-${i}`,
        header: "",
        align: "center",
        mono: true,
        cell: (c) => {
          const bin = c.row.original.bins[i];
          return (
            <span className={cn("font-semibold", alertTextClass[bin.alert])} title={`active ${bin.active} · proposed ${bin.proposed}`}>
              {bin.combined}
            </span>
          );
        },
      }),
    ),
    {
      id: "order",
      header: "",
      cell: (c) => (
        <span className="inline-flex gap-1">
          <Button size="sm" variant="ghost" aria-label={`Move ${c.row.original.sector_id} up`} onClick={() => move(c.row.original.sector_id, -1)}>
            <ArrowUp />
          </Button>
          <Button size="sm" variant="ghost" aria-label={`Move ${c.row.original.sector_id} down`} onClick={() => move(c.row.original.sector_id, 1)}>
            <ArrowDown />
          </Button>
        </span>
      ),
    },
  ];

  const footer = (
    <tr>
      <td colSpan={2} />
      {shownBins.map((b) => (
        <td key={b.start} className="px-1 py-2 text-center font-mono text-xs text-ink-3">
          {hhmmZulu(b.start)}
        </td>
      ))}
      <td />
    </tr>
  );

  return (
    <div className="flex flex-col gap-3">
      <FilterBar>
        <label className="flex items-center gap-2 text-xs font-semibold text-ink-2">
          Time Range {hours} h
          <input
            type="range"
            aria-label="Time Range"
            min={TIME_RANGES[0]}
            max={TIME_RANGES[TIME_RANGES.length - 1]}
            step={1}
            value={hours}
            onChange={(e) => setHours(Number(e.target.value))}
            className="w-32 accent-brand"
          />
        </label>
        <Select aria-label="Show if alerted in next" size="sm" value={alertHours} onChange={(e) => setAlertHours(Number(e.target.value))}>
          {ALERT_WINDOWS.map((h) => (
            <option key={h} value={h}>
              {h === 0 ? "All sectors" : `Alerted in next ${h.toFixed(2)} h`}
            </option>
          ))}
        </Select>
      </FilterBar>

      <QueryState
        isLoading={monitor.isLoading}
        isError={monitor.isError}
        onRetry={() => monitor.refetch()}
        isEmpty={monitor.data != null && allRows.length === 0}
        empty="No sector data yet: no sector volumes are loaded for this ARTCC."
        className="rounded-md border border-line"
      >
        {visible.length === 0 ? (
          <EmptyState title="Nothing alerting" className="rounded-md border border-line">
            No sector is amber or red in the next {alertHours} h.
          </EmptyState>
        ) : (
          <DataTable
            label={`${artcc} Airspace Monitor`}
            columns={columns}
            data={visible}
            getRowId={(r) => r.sector_id}
            sortable={false}
            rowCap={Infinity}
            footer={footer}
            onRowContextMenu={(row, e) => {
              if (!editable) return; // a control you cannot use does not respond
              e.preventDefault();
              setMenu({sector: row, x: e.clientX, y: e.clientY});
            }}
          />
        )}
      </QueryState>

      <DropdownMenu open={menu != null} onOpenChange={(open) => !open && setMenu(null)}>
        <DropdownMenuTrigger asChild>
          <span aria-hidden className="fixed size-0" style={{left: menu?.x ?? 0, top: menu?.y ?? 0}} />
        </DropdownMenuTrigger>
        {menu && (
          <DropdownMenuContent align="start">
            <DropdownMenuLabel>{menu.sector.sector_id}</DropdownMenuLabel>
            {ordered
              .filter((r) => r.sector_id !== menu.sector.sector_id)
              .map((r) => (
                <DropdownMenuItem key={r.sector_id} onSelect={() => consolidate.mutate({sectorId: menu.sector.sector_id, target: r.sector_id})}>
                  Consolidate into {r.sector_id}
                </DropdownMenuItem>
              ))}
            {menu.sector.consolidated.length > 0 && <DropdownMenuSeparator />}
            {menu.sector.consolidated.map((source) => (
              <DropdownMenuItem key={source} onSelect={() => release.mutate(source)}>
                Release {source}
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => consolidateAll.mutate({target: menu.sector.sector_id, mode: "all"})}>
              All into {menu.sector.sector_id}
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() => consolidateAll.mutate({target: menu.sector.sector_id, mode: "except_consolidated"})}
            >
              All into {menu.sector.sector_id} except consolidated
            </DropdownMenuItem>
          </DropdownMenuContent>
        )}
      </DropdownMenu>
    </div>
  );
}

/** One first-tier neighbour, collapsed until opened; its table only loads once it is (#712). */
function NeighbourTable({artcc}: {artcc: string}) {
  const [open, setOpen] = useState(false);
  return (
    <section className="flex flex-col gap-2">
      <Button
        variant="ghost"
        size="sm"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="self-start font-mono"
      >
        {open ? <ChevronDown /> : <ChevronRight />}
        {artcc}
      </Button>
      {open && <MonitorTable artcc={artcc} viewOnly />}
    </section>
  );
}

/** Airspace Monitor (#601): pick an ARTCC, watch its sectors load against their MAPs. */
export function MonitorPage() {
  usePageHeader({
    subtitle:
      "Each sector's peak occupancy per 15 minutes, against its Monitor Alert Parameter: red when the airborne peak exceeds it, amber when departures still holding would.",
  });
  const facilities = useFacilities();
  const artccs = useMemo(
    () => (facilities.data ?? []).filter((f) => f.active).sort((a, b) => a.id.localeCompare(b.id)),
    [facilities.data],
  );
  const [artcc, setArtcc] = useLocalStorage<string>("ois.monitor.artcc", "");
  const neighbours = useMonitorNeighbours(artcc);
  useEffect(() => {
    if (!artcc && artccs.length) setArtcc(artccs[0].id);
  }, [artcc, artccs, setArtcc]);

  return (
    <div className="flex flex-col gap-3">
      <FilterBar>
        <Select aria-label="ARTCC" size="sm" value={artcc} onChange={(e) => setArtcc(e.target.value)}>
          {artccs.map((f) => (
            <option key={f.id} value={f.id}>
              {f.id}
            </option>
          ))}
        </Select>
      </FilterBar>
      {artcc && <MonitorTable key={artcc} artcc={artcc} />}
      {(neighbours.data ?? []).map((n) => (
        <NeighbourTable key={`${artcc}:${n}`} artcc={n} />
      ))}
    </div>
  );
}

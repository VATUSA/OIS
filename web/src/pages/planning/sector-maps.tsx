import {useEffect, useMemo, useState} from "react";
import {type DataColumn, DataTable, FilterBar, Input, Select} from "@ois/ui";
import {Gauge, Hash, Tag} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useFacilities} from "@/lib/admin";
import {mapEdit, type SectorMap, useSectorMaps, useSetSectorMap} from "@/lib/sector-maps";

/** A sector's MAP: a number, or — when the caller may edit this ARTCC — an input that writes on Enter
 * or blur only through `mapEdit`, and otherwise snaps back. Opening the page never writes. */
function MapCell({artcc, row, editable}: {artcc: string; row: SectorMap; editable: boolean}) {
  const setMap = useSetSectorMap(artcc);
  const [draft, setDraft] = useState(String(row.map));
  useEffect(() => setDraft(String(row.map)), [row.map]);

  if (!editable) return <span className="font-mono">{row.map}</span>;

  const commit = () => {
    const value = mapEdit(draft, row.map);
    if (value === null) setDraft(String(row.map));
    else setMap.mutate({sectorId: row.sector_id, map: value});
  };

  return (
    <Input
      aria-label={`${row.sector_id} alert parameter`}
      className="h-8 w-20 font-mono"
      inputMode="numeric"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
}

export function SectorMapsPage() {
  usePageHeader({
    subtitle:
      "Monitor Alert Parameters: the per-sector count the Airspace Monitor colours against. Shared by everyone watching the ARTCC; facility TMUs edit only their own.",
  });
  const facilities = useFacilities();
  const artccs = useMemo(
    () => (facilities.data ?? []).filter((f) => f.active).sort((a, b) => a.id.localeCompare(b.id)),
    [facilities.data],
  );
  const [artcc, setArtcc] = useState<string>();
  useEffect(() => {
    if (!artcc && artccs.length) setArtcc(artccs[0].id);
  }, [artcc, artccs]);

  const maps = useSectorMaps(artcc);
  const editable = maps.data?.editable ?? false;

  const columns = useMemo<DataColumn<SectorMap>[]>(
    () => [
      {accessorKey: "sector_id", header: "Sector", icon: Hash, cell: (c) => <span className="font-mono">{c.row.original.sector_id}</span>},
      {accessorKey: "name", header: "Name", icon: Tag, cell: (c) => c.row.original.name ?? "—"},
      {
        accessorKey: "map",
        header: "MAP",
        icon: Gauge,
        cell: (c) => <MapCell artcc={artcc ?? ""} row={c.row.original} editable={editable} />,
      },
      {
        accessorKey: "overridden",
        header: "Source",
        cell: (c) => (
          <span className="text-xs text-ink-2">{c.row.original.overridden ? "override" : "default"}</span>
        ),
      },
    ],
    [artcc, editable],
  );

  return (
    <section className="flex flex-col gap-4">
      <FilterBar>
        <Select aria-label="ARTCC" size="sm" value={artcc ?? ""} onChange={(e) => setArtcc(e.target.value)}>
          {artccs.map((f) => (
            <option key={f.id} value={f.id}>
              {f.id} — {f.name}
            </option>
          ))}
        </Select>
        {maps.data && (
          <span className="text-xs text-ink-2">Default {maps.data.default_map}; typing it resets an override.</span>
        )}
      </FilterBar>

      <DataTable
        label="Sector alert parameters"
        columns={columns}
        data={maps.data?.sectors ?? []}
        getRowId={(r) => r.sector_id}
        isLoading={maps.isLoading}
        isError={!maps.data && maps.isError}
        onRetry={() => maps.refetch()}
        empty="No sectors imported for this ARTCC."
      />
    </section>
  );
}

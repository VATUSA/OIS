import {useEffect, useMemo, useState} from "react";
import {type DataColumn, DataTable, FilterBar, Select} from "@ois/ui";
import {Gauge, Hash, Tag} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useFacilities} from "@/lib/admin";
import {SectorMapCell} from "@/components/sector-map-cell";
import {type SectorMap, useSectorMaps} from "@/lib/sector-maps";

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
        cell: (c) => (
          <SectorMapCell artcc={artcc ?? ""} sectorId={c.row.original.sector_id} map={c.row.original.map} editable={editable} />
        ),
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

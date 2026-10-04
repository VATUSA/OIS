import {useEffect, useState} from "react";
import {Input} from "@ois/ui";

import {mapEdit, useSetSectorMap} from "@/lib/sector-maps";

/**
 * A sector's MAP (#598): a number, or — when the caller may edit this ARTCC — an input that writes on
 * Enter or blur only through `mapEdit`, and otherwise snaps back. Opening a page never writes. Shared
 * by the alert-parameter editor and the Airspace Monitor (#601).
 */
export function SectorMapCell({
  artcc,
  sectorId,
  map,
  editable,
}: {
  artcc: string;
  sectorId: string;
  map: number;
  editable: boolean;
}) {
  const setMap = useSetSectorMap(artcc);
  const [draft, setDraft] = useState(String(map));
  useEffect(() => setDraft(String(map)), [map]);

  if (!editable) return <span className="font-mono">{map}</span>;

  const commit = () => {
    const value = mapEdit(draft, map);
    if (value === null) setDraft(String(map));
    else setMap.mutate({sectorId, map: value});
  };

  return (
    <Input
      aria-label={`${sectorId} alert parameter`}
      className="h-8 w-20 font-mono"
      inputMode="numeric"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
}

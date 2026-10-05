import {PolygonLayer} from "@deck.gl/layers";

import type {SectorVolume} from "@/lib/sectors";
import type {MapPalette} from "../lib/colors";
import {type LatLng, sanitizeRings, toDeckPath} from "../lib/geo";
import type {RGBA} from "../lib/types";

export const SECTOR_LAYER_ID = "airspace-sectors";

/**
 * The four strata, in the order the toggles and legend show them. Each takes one `--series-N` token,
 * so the map adds no colour of its own (DESIGN.md: tokens only).
 */
export const SECTOR_TIERS = [
  { tier: "low", label: "Low", series: 0 },
  { tier: "high", label: "High", series: 1 },
  { tier: "ultra_high", label: "Ultra high", series: 2 },
  { tier: "approach", label: "Approach", series: 3 },
] as const;

export type SectorTier = (typeof SECTOR_TIERS)[number]["tier"];

/** A volume's rings as deck draws them: topologically sound (a bridged ring split, a crossing one
 * dropped — #481) and flipped to `[lon, lat]`. */
export const sectorPolygons = (v: SectorVolume): [number, number][][] =>
  sanitizeRings(v.rings as LatLng[][]).map(toDeckPath);

/**
 * The admin sector map's one layer: every volume in a selected stratum, outlined and faintly filled in
 * its tier's colour. Pickable, for the hover card. Strata stack over the same footprint, so showing
 * one at a time is what makes them readable.
 */
export function buildSectorLayer(volumes: SectorVolume[], strata: readonly string[], palette: MapPalette) {
  const shown = new Set(strata);
  const colorOf = (tier: string) => {
    const t = SECTOR_TIERS.find((x) => x.tier === tier);
    return t ? (palette.series[t.series] ?? palette.muted) : palette.muted;
  };
  // One deck polygon per sound ring, so a split bridged ring draws as its separate parts.
  const data = volumes
    .filter((v) => shown.has(v.tier))
    .flatMap((v) => sectorPolygons(v).map((polygon) => ({ volume: v, polygon })));
  return new PolygonLayer<(typeof data)[number]>({
    id: SECTOR_LAYER_ID,
    data,
    pickable: true,
    stroked: true,
    filled: true,
    getPolygon: (d) => d.polygon,
    getLineColor: (d) => [...colorOf(d.volume.tier), 220] as RGBA,
    getFillColor: (d) => [...colorOf(d.volume.tier), 28] as RGBA,
    getLineWidth: 1,
    lineWidthUnits: "pixels",
    lineWidthMinPixels: 1,
    updateTriggers: { getLineColor: [palette], getFillColor: [palette] },
  });
}

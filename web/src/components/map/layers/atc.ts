import {GeoJsonLayer, PolygonLayer, ScatterplotLayer} from "@deck.gl/layers";
import type {Layer} from "@deck.gl/core";

import {ATC_COLORS, hexToRgb} from "../lib/colors";
import type {RGBA} from "../lib/types";

/** ATC board subset the map renders (from the /flow/atc endpoint). */
export interface AtcData {
  airports: { icao: string; lat: number; lon: number; positions: AtcPositionLite[] }[];
  centers: { id: string; positions: AtcPositionLite[] }[];
  tracons: {
    id: string;
    name?: string | null;
    rings: number[][][]; // rings of [lat, lon]
    circle?: number[] | null; // [lat, lon]
    label?: number[] | null; // [lat, lon]
    positions: AtcPositionLite[];
  }[];
}
export interface AtcPositionLite {
  callsign: string;
  frequency: string;
  kind: string;
  atis_code?: string | null;
}

const CTR = hexToRgb(ATC_COLORS.CTR);
const APP = hexToRgb(ATC_COLORS.APP);

/**
 * ATC area shading: online centers get their bundled ARTCC polygon shaded teal; TRACONs get their
 * matched SimAware rings shaded orange, or a ~25 NM circle fallback. Badges + area id labels are HTML
 * markers (see the markers/ components), not drawn here.
 */
export function buildAtcLayers(atc: AtcData, boundaries: GeoJSON.FeatureCollection): Layer[] {
  const layers: Layer[] = [];

  // Center (ARTCC) areas — filter the bundled boundaries to the online centers.
  const online = new Set(atc.centers.map((c) => c.id.toUpperCase()));
  const centerFeatures = boundaries.features.filter((f) =>
    online.has(String(f.properties?.id ?? "").toUpperCase()),
  );
  if (centerFeatures.length > 0) {
    layers.push(
      new GeoJsonLayer({
        id: "atc-centers",
        data: { type: "FeatureCollection", features: centerFeatures } as GeoJSON.FeatureCollection,
        stroked: true,
        filled: true,
        getLineColor: [...CTR, 140] as RGBA,
        getFillColor: [...CTR, 20] as RGBA,
        getLineWidth: 1.5,
        lineWidthUnits: "pixels",
        lineWidthMinPixels: 1,
      }),
    );
  }

  // TRACON polygon rings (rings are [lat, lon]; deck polygons want [lon, lat]).
  const ringPolys = atc.tracons
    .filter((t) => !t.circle && t.rings.length > 0)
    .flatMap((t) => t.rings.map((ring) => ({ contour: ring.map(([lat, lon]) => [lon, lat]) })));
  if (ringPolys.length > 0) {
    layers.push(
      new PolygonLayer<{ contour: number[][] }>({
        id: "atc-tracon-polys",
        data: ringPolys,
        getPolygon: (d) => d.contour,
        stroked: true,
        filled: true,
        getLineColor: [...APP, 180] as RGBA,
        getFillColor: [...APP, 25] as RGBA,
        getLineWidth: 1.5,
        lineWidthUnits: "pixels",
        lineWidthMinPixels: 1,
      }),
    );
  }

  // TRACON circle fallbacks (~25 NM).
  const circles = atc.tracons
    .filter((t) => t.circle)
    .map((t) => ({ pos: [t.circle![1], t.circle![0]] as [number, number] }));
  if (circles.length > 0) {
    layers.push(
      new ScatterplotLayer<{ pos: [number, number] }>({
        id: "atc-tracon-circles",
        data: circles,
        getPosition: (d) => d.pos,
        getRadius: 46300, // ~25 NM in metres
        radiusUnits: "meters",
        stroked: true,
        filled: true,
        getLineColor: [...APP, 150] as RGBA,
        getFillColor: [...APP, 15] as RGBA,
        lineWidthUnits: "pixels",
        getLineWidth: 1.5,
        lineWidthMinPixels: 1,
      }),
    );
  }

  return layers;
}

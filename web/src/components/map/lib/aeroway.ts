import {AEROWAY_COLORS, type Theme} from "./constants";

/** Minimal MapLibre surface we touch — avoids depending on maplibre-gl's exported types. */
export interface StyleMap {
  getSource(id: string): unknown;
  getLayer(id: string): unknown;
  addLayer(layer: Record<string, unknown>): void;
}

/**
 * Draw airport layouts (runways, taxiways, aprons) straight from the OSM `aeroway` data already in
 * the CARTO vector tiles — the base style renders it in near-black (invisible), so we add our own
 * visible layers instead. Free, no extra requests, appears once you zoom into a field (z≥10).
 * Idempotent: safe to call on every `styledata` (re-added after a theme swap wipes the style).
 */
export function ensureAeroway(map: StyleMap, theme: Theme): void {
  try {
    if (!map.getSource("carto") || map.getLayer("ois-aeroway-fill")) return;
    const c = AEROWAY_COLORS[theme];
    const base = { source: "carto", "source-layer": "aeroway" } as const;
    map.addLayer({
      ...base,
      id: "ois-aeroway-fill",
      type: "fill",
      minzoom: 10,
      filter: ["==", ["geometry-type"], "Polygon"],
      paint: { "fill-color": c.fill, "fill-opacity": 0.6 },
    });
    map.addLayer({
      ...base,
      id: "ois-aeroway-taxiway",
      type: "line",
      minzoom: 12,
      filter: ["all", ["==", ["geometry-type"], "LineString"], ["==", ["get", "class"], "taxiway"]],
      paint: {
        "line-color": c.taxiway,
        "line-width": ["interpolate", ["linear"], ["zoom"], 12, 0.6, 14, 1.5, 16, 4],
      },
    });
    map.addLayer({
      ...base,
      id: "ois-aeroway-runway",
      type: "line",
      minzoom: 10,
      filter: ["all", ["==", ["geometry-type"], "LineString"], ["==", ["get", "class"], "runway"]],
      paint: {
        "line-color": c.runway,
        "line-width": ["interpolate", ["linear"], ["zoom"], 10, 1.2, 13, 4, 15, 9, 16, 13],
      },
    });
  } catch {
    // Style not fully ready yet — a later `styledata` event retries.
  }
}

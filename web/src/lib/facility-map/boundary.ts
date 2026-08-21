/** Single-facility ARTCC boundary helpers over the bundled boundary GeoJSON (no endpoint needed). */

import boundariesGeo from "@/assets/artcc-boundaries.json";

const BOUNDARIES = boundariesGeo as GeoJSON.FeatureCollection;

/** Every ARTCC boundary — the national overview shown when no single facility is selected. */
export const ALL_BOUNDARIES: GeoJSON.FeatureCollection = BOUNDARIES;

const featureId = (f: GeoJSON.Feature): string | undefined =>
  (f.properties as { id?: string } | null)?.id;

/** The boundary feature for an ARTCC id, or null if we have no polygon for it. */
export function facilityFeature(id: string): GeoJSON.Feature | null {
  return BOUNDARIES.features.find((f) => featureId(f) === id) ?? null;
}

/** All boundary vertices as deck `[lon, lat]` pairs — feed to `camera.fitBounds`. */
export function facilityPoints(feature: GeoJSON.Feature): [number, number][] {
  const out: [number, number][] = [];
  const ring = (r: GeoJSON.Position[]) => {
    for (const p of r) out.push([p[0], p[1]]);
  };
  const g = feature.geometry;
  if (g.type === "Polygon") g.coordinates.forEach(ring);
  else if (g.type === "MultiPolygon") g.coordinates.forEach((poly) => poly.forEach(ring));
  return out;
}

/** Wrap a single feature as a FeatureCollection for the boundary layer. */
export function facilityCollection(feature: GeoJSON.Feature): GeoJSON.FeatureCollection {
  return { type: "FeatureCollection", features: [feature] };
}

/** ARTCC ids that actually have a boundary polygon — used to filter the facility picker. */
export const BOUNDARY_IDS: Set<string> = new Set(
  BOUNDARIES.features.map(featureId).filter((x): x is string => !!x),
);

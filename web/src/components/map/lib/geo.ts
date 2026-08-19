/** Shared geo helpers for the map (lat/lon math; deck draws in [lon, lat] order). */

export type LatLng = [number, number];

/** Great-circle distance in nautical miles between two [lat, lon] points. */
export function haversine(a: LatLng, b: LatLng): number {
  const R = 3440.065; // nm
  const dLat = ((b[0] - a[0]) * Math.PI) / 180;
  const dLon = ((b[1] - a[1]) * Math.PI) / 180;
  const la1 = (a[0] * Math.PI) / 180;
  const la2 = (b[0] * Math.PI) / 180;
  const h =
    Math.sin(dLat / 2) ** 2 + Math.cos(la1) * Math.cos(la2) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(h));
}

/** Total length of a [lat, lon] polyline, nm. */
export function lineNm(pts: LatLng[]): number {
  let d = 0;
  for (let i = 0; i < pts.length - 1; i++) d += haversine(pts[i], pts[i + 1]);
  return d;
}

/** The point halfway along a [lat, lon] polyline by arc length (the true visual center). */
export function midpointOf(pts: LatLng[]): LatLng {
  if (pts.length < 2) return pts[0];
  const segs = pts.slice(1).map((p, i) => haversine(pts[i], p));
  let half = segs.reduce((a, b) => a + b, 0) / 2;
  for (let i = 0; i < segs.length; i++) {
    if (half <= segs[i]) {
      const f = segs[i] ? half / segs[i] : 0;
      return [pts[i][0] + (pts[i + 1][0] - pts[i][0]) * f, pts[i][1] + (pts[i + 1][1] - pts[i][1]) * f];
    }
    half -= segs[i];
  }
  return pts[pts.length - 1];
}

/** Normalize a longitude into [-180, 180). */
export const normalizeLng = (lng: number): number => ((((lng + 180) % 360) + 360) % 360) - 180;

/** Normalize each [lat, lon] vertex's longitude into [-180, 180) (for storage). */
export const normPoints = (pts: LatLng[]): LatLng[] => pts.map(([lat, lng]) => [lat, normalizeLng(lng)]);

/** Centroid [lat, lon] of a ring of [lat, lon] points. */
export function centroid(ring: LatLng[]): LatLng {
  let x = 0;
  let y = 0;
  for (const [lat, lon] of ring) {
    x += lon;
    y += lat;
  }
  return [y / ring.length, x / ring.length];
}

/** Convert a [lat, lon] polyline to deck's [lon, lat] path order. */
export const toDeckPath = (pts: LatLng[]): [number, number][] => pts.map(([lat, lon]) => [lon, lat]);

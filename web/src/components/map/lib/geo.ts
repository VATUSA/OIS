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

/** Convert a single [lat, lon] point to deck's [lon, lat] order. */
export const toDeckPoint = ([lat, lon]: LatLng): [number, number] => [lon, lat];

// --- ring topology (VATUSA/OIS#481) ---------------------------------------------------------------
//
// Every existing guard on these rings is per-vertex: finite, on-globe, not a far outlier. A ring can
// pass all of that and still be two lobes joined by a zero-width bridge — which is what the bundled
// ZNY boundary was, and earcut tessellates it into triangles spanning *between* the lobes, the
// map-wide wedges of #477. Vertex checks cannot see it; only topology can.
//
// Axis order is irrelevant here, so these work on both the TRACON path's [lat, lon] rings and
// GeoJSON's [lon, lat] ones. The callers keep their own on-globe checks, which catch a different
// class of problem.

/** A vertex in whichever axis order the caller uses — topology does not care which. */
type Pt = [number, number];

const vertexKey = (p: Pt) => `${p[0]},${p[1]}`;

/** A ring without its closing repeat: GeoJSON closes its rings, the TRACON feed does not. */
function openRing(ring: Pt[]): Pt[] {
  const n = ring.length;
  return n > 1 && vertexKey(ring[0]) === vertexKey(ring[n - 1]) ? ring.slice(0, -1) : ring.slice();
}

/**
 * Split a ring wherever it revisits a vertex. A revisited vertex is where two loops were flattened
 * into one: the span between the two visits is a closed loop of its own, and what remains either
 * side is the rest of the figure. Recurses, so a chain of bridged lobes comes apart in one pass.
 *
 * Degenerate remnants (the bridge itself, a two-vertex spur) fall out with fewer than three
 * vertices; the caller drops them, and they enclose no area so nothing is lost.
 */
function splitBridged(open: Pt[]): Pt[][] {
  const seen = new Map<string, number>();
  for (let j = 0; j < open.length; j++) {
    const k = vertexKey(open[j]);
    const i = seen.get(k);
    if (i !== undefined) {
      const loop = open.slice(i, j);
      const rest = [...open.slice(0, i), ...open.slice(j)];
      return [...splitBridged(loop), ...splitBridged(rest)];
    }
    seen.set(k, j);
  }
  return [open];
}

/** Which side of segment `a`→`b` point `c` lies on; 0 when collinear. */
const turn = (a: Pt, b: Pt, c: Pt) =>
  Math.sign((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]));

/** Whether two segments cross at a point interior to both — touching at a shared endpoint is not a
 * crossing, and collinear overlap is deliberately not treated as one (it encloses no area). */
function segmentsCross(p1: Pt, p2: Pt, p3: Pt, p4: Pt): boolean {
  const d1 = turn(p3, p4, p1);
  const d2 = turn(p3, p4, p2);
  const d3 = turn(p1, p2, p3);
  const d4 = turn(p1, p2, p4);
  return d1 !== 0 && d2 !== 0 && d3 !== 0 && d4 !== 0 && d1 !== d2 && d3 !== d4;
}

/**
 * Whether any two non-adjacent edges of a closed ring cross. O(n²), which is free here: these rings
 * top out around 115 vertices.
 *
 * Unlike a bridged ring this cannot be split into meaningful lobes — there is no shared vertex to
 * split at — so the caller rejects it rather than trying to repair it.
 */
function hasSelfCrossing(open: Pt[]): boolean {
  const n = open.length;
  if (n < 4) return false;
  for (let i = 0; i < n; i++) {
    const a1 = open[i];
    const a2 = open[(i + 1) % n];
    for (let j = i + 1; j < n; j++) {
      // Skip adjacent edges, and the first/last pair which are adjacent through the closing vertex.
      if (j === i || j === i + 1 || (i === 0 && j === n - 1)) continue;
      if (segmentsCross(a1, a2, open[j], open[(j + 1) % n])) return true;
    }
  }
  return false;
}

/** A ring's topology verdict: the sound rings it yields, or why it could not be used. */
export type RingTopology = {ok: true; rings: Pt[][]} | {ok: false; reason: string};

/**
 * Make one ring topologically sound: split it where it revisits a vertex, drop the degenerate
 * remnants, and reject what is left if any part still crosses itself.
 *
 * Returns the rings **open** (no closing repeat), which is what both callers want to re-close in
 * their own convention.
 */
export function sanitizeRingTopology(ring: Pt[]): RingTopology {
  const open = openRing(ring);
  if (open.length < 3) return {ok: false, reason: "fewer than three distinct vertices"};
  const parts = splitBridged(open).filter((r) => r.length >= 3);
  if (parts.length === 0) return {ok: false, reason: "no part enclosed an area"};
  if (parts.some(hasSelfCrossing)) return {ok: false, reason: "edges cross without sharing a vertex"};
  return {ok: true, rings: parts};
}

/**
 * Topologically sound `[lat, lon]` rings, for the TRACON path. A bridged ring becomes several — a
 * multi-lobe TRACON then draws each lobe correctly instead of a wedge between them — and a ring that
 * cannot be repaired is dropped, as malformed rings already were.
 */
export function sanitizeRings(rings: LatLng[][]): LatLng[][] {
  return rings.flatMap((ring) => {
    const verdict = sanitizeRingTopology(ring as Pt[]);
    return verdict.ok ? (verdict.rings as LatLng[][]) : [];
  });
}

/** Close an open ring the way GeoJSON wants it. */
const closeRing = (ring: Pt[]): Pt[] => [...ring, ring[0]];

/** Sound polygons for one feature, or `null` when none survive. `changed` reports whether anything
 * was split or dropped, so an untouched collection can be returned by identity. */
function soundPolygons(polys: Pt[][][]): {polys: Pt[][][]; changed: boolean} | null {
  let changed = false;
  const out: Pt[][][] = [];
  for (const poly of polys) {
    const verdict = sanitizeRingTopology(poly[0]);
    if (!verdict.ok) {
      changed = true;
      continue;
    }
    const outers = verdict.rings;
    if (outers.length > 1) changed = true;
    // Holes ride along only when the outer ring stayed whole: once it splits there is no way to say
    // which lobe a hole belongs to without a point-in-polygon test, and this asset has no holes at
    // all. A hole that is itself unsound is dropped and its outer kept.
    const holes = outers.length === 1 ? poly.slice(1).filter((h) => sanitizeRingTopology(h).ok) : [];
    if (holes.length !== poly.length - 1) changed = true;
    for (const outer of outers) out.push([closeRing(outer), ...holes.map((h) => closeRing(openRing(h)))]);
  }
  return out.length > 0 ? {polys: out, changed} : null;
}

const sanitizedBoundaries = new WeakMap<GeoJSON.FeatureCollection, GeoJSON.FeatureCollection>();

/**
 * A boundary collection with every fillable ring made topologically sound — for the ARTCC shading
 * and outline paths, which pass bundled GeoJSON straight to a filled `GeoJsonLayer`.
 *
 * A `Polygon` whose ring splits becomes a `MultiPolygon`, so a bridged boundary draws as the separate
 * areas it actually is; a feature with nothing salvageable is dropped rather than drawn wrong.
 *
 * Memoized on the collection, because the bundled asset is a module constant and these layers
 * rebuild whenever the palette or selection changes. An unchanged collection is returned by
 * identity, so downstream memoization still sees the same object.
 */
export function sanitizeBoundaries(fc: GeoJSON.FeatureCollection): GeoJSON.FeatureCollection {
  const cached = sanitizedBoundaries.get(fc);
  if (cached) return cached;

  let changed = false;
  const features: GeoJSON.Feature[] = [];
  for (const f of fc.features) {
    const g = f.geometry;
    if (g?.type !== "Polygon" && g?.type !== "MultiPolygon") {
      features.push(f);
      continue;
    }
    const polys = (g.type === "Polygon" ? [g.coordinates] : g.coordinates) as Pt[][][];
    const sound = soundPolygons(polys);
    if (!sound) {
      changed = true;
      continue;
    }
    if (!sound.changed) {
      features.push(f);
      continue;
    }
    changed = true;
    features.push({
      ...f,
      geometry:
        sound.polys.length === 1
          ? {type: "Polygon", coordinates: sound.polys[0] as GeoJSON.Position[][]}
          : {type: "MultiPolygon", coordinates: sound.polys as GeoJSON.Position[][][]},
    });
  }

  const out = changed ? {...fc, features} : fc;
  sanitizedBoundaries.set(fc, out);
  return out;
}

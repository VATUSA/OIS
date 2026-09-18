import {GeoJsonLayer, PolygonLayer, ScatterplotLayer} from "@deck.gl/layers";
import type {Layer} from "@deck.gl/core";

import {ATC_COLORS, type MapPalette, readMapPalette} from "../lib/colors";
import {toDeckPath, toDeckPoint, type LatLng} from "../lib/geo";
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
  name: string;
  rating: number;
  logon_time: string;
  atis_code?: string | null;
}

/** A `[lat, lon]` pair with both values finite and on the globe (a transposed `[lon, lat]` for
 * most of the US fails the latitude bound). */
function isValidPoint(p: number[] | null | undefined): p is number[] {
  return (
    !!p &&
    p.length >= 2 &&
    Number.isFinite(p[0]) &&
    Number.isFinite(p[1]) &&
    Math.abs(p[0]) <= 90 &&
    Math.abs(p[1]) <= 180
  );
}

/** A vertex further than this (degrees, either axis) from its ring's median vertex is garbage: real
 * TRACON boundaries span a few degrees at most (VATUSA/OIS#318). */
const MAX_RING_OUTLIER_DEG = 5;

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

/** A polygon ring needs at least 3 valid vertices and no outlier — fewer, a stray non-finite value,
 * or one far-off vertex triangulates into a huge stretched wedge across the map instead of failing
 * visibly, which is exactly the "stretched boundary" glitch this guards against. */
function isValidRing(ring: number[][]): boolean {
  if (ring.length < 3 || !ring.every(isValidPoint)) return false;
  const [mlat, mlon] = ringMedian(ring);
  return ring.every((p) => Math.abs(p[0] - mlat) <= MAX_RING_OUTLIER_DEG && Math.abs(p[1] - mlon) <= MAX_RING_OUTLIER_DEG);
}

/** The per-axis median `[lat, lon]` of a ring's vertices — where it really is, unmoved by one stray vertex. */
function ringMedian(ring: number[][]): [number, number] {
  return [median(ring.map((p) => p[0])), median(ring.map((p) => p[1]))];
}

/**
 * ATC area shading: online centers get their bundled ARTCC polygon shaded teal; TRACONs get their
 * matched SimAware rings shaded orange, or a ~25 NM circle fallback. Badges + area id labels are HTML
 * markers (see the markers/ components), not drawn here.
 */
export function buildAtcLayers(
  atc: AtcData,
  boundaries: GeoJSON.FeatureCollection,
  palette: MapPalette = readMapPalette(),
): Layer[] {
  const CTR = palette.atc.CTR;
  const APP = palette.atc.APP;
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

  // TRACON polygon rings (rings are [lat, lon]; deck polygons want [lon, lat]). A malformed ring
  // is dropped on its own — other valid rings on the same TRACON still render; a TRACON left with
  // none falls back to a circle at its label.
  const polygonTracons = atc.tracons
    .filter((t) => !t.circle && t.rings.length > 0)
    .map((t) => ({ t, rings: t.rings.filter(isValidRing) }));
  const ringPolys = polygonTracons.flatMap(({ rings }) =>
    rings.map((ring) => ({ contour: toDeckPath(ring as LatLng[]) })),
  );
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
  // A TRACON whose rings were all dropped keeps a circle: at its label, else where its first ring
  // really sits (its median vertex, which one stray vertex can't drag across the map).
  const circleCenters = [
    ...atc.tracons.map((t) => t.circle),
    ...polygonTracons
      .filter(({ rings }) => rings.length === 0)
      .map(({ t }) => (isValidPoint(t.label) ? t.label : ringMedian(t.rings[0].filter(isValidPoint)))),
  ];
  const circles = circleCenters
    .filter(isValidPoint)
    .map((c) => ({ pos: toDeckPoint(c as LatLng) }));
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

// --- Shared anchors (used by both the HTML pill markers and the pickable hover targets) ---

/** A placed ATC label: an airport badge stack, or a center/TRACON id pill. */
export type AtcAnchor =
  | { type: "airport"; lat: number; lon: number; icao: string; positions: AtcPositionLite[] }
  | {
      type: "area";
      lat: number;
      lon: number;
      id: string;
      name?: string | null;
      color: string;
      positions: AtcPositionLite[];
    };

function ringsCentroid(rings: number[][][]): [number, number] | null {
  const outer = rings[0];
  if (!outer || outer.length === 0) return null;
  let slat = 0;
  let slon = 0;
  for (const [lat, lon] of outer) {
    slat += lat;
    slon += lon;
  }
  return [slat / outer.length, slon / outer.length];
}

/** Centroid [lat, lon] of a boundary feature's outer ring (GeoJSON coords are [lon, lat]). */
function featureCentroid(feat: GeoJSON.Feature): [number, number] | null {
  const geom = feat.geometry;
  const outer =
    geom.type === "Polygon"
      ? geom.coordinates[0]
      : geom.type === "MultiPolygon"
        ? geom.coordinates[0]?.[0]
        : null;
  if (!outer || outer.length === 0) return null;
  let sx = 0;
  let sy = 0;
  for (const [lon, lat] of outer as number[][]) {
    sx += lon;
    sy += lat;
  }
  return [sy / outer.length, sx / outer.length];
}

/** Compute the on-map anchor for every ATC label (airport badge, center pill, TRACON pill). */
export function computeAtcAnchors(atc: AtcData, boundaries: GeoJSON.FeatureCollection): AtcAnchor[] {
  const anchors: AtcAnchor[] = [];
  for (const ap of atc.airports) {
    // No DEL/GND/TWR/ATIS pill is drawn for it (`AtcBadge`), so a hover target would float over empty map.
    if (atcBadgeKinds(ap.positions).length === 0) continue;
    anchors.push({ type: "airport", lat: ap.lat, lon: ap.lon, icao: ap.icao, positions: ap.positions });
  }
  const byId = new Map<string, GeoJSON.Feature>();
  for (const f of boundaries.features) {
    const id = String(f.properties?.id ?? "").toUpperCase();
    if (id) byId.set(id, f);
  }
  for (const c of atc.centers) {
    const feat = byId.get(c.id.toUpperCase());
    const at = feat ? featureCentroid(feat) : null;
    if (at) anchors.push({ type: "area", lat: at[0], lon: at[1], id: c.id, color: ATC_COLORS.CTR, positions: c.positions });
  }
  for (const t of atc.tracons) {
    // `??` only falls through on null/undefined, not on a present-but-invalid point (e.g. `[]` or
    // `[NaN, NaN]`), so each candidate is validated explicitly instead of relying on nullish-coalescing.
    const candidates: (number[] | null | undefined)[] = [t.label, t.circle, ringsCentroid(t.rings)];
    const at = candidates.find(isValidPoint);
    if (at) {
      anchors.push({ type: "area", lat: at[0], lon: at[1], id: t.id, name: t.name, color: ATC_COLORS.APP, positions: t.positions });
    }
  }
  return anchors;
}

/** The header line for an anchor's hover card (ICAO, or `ID · Name`). */
export const anchorHeader = (a: AtcAnchor) =>
  a.type === "airport" ? a.icao : `${a.id}${a.name ? " · " + a.name : ""}`;

/** Which stacked pills `AtcBadge` (`markers/AtcMarkers.tsx`) renders for an airport, in order.
 * Shared with the hover hit-area below so the two can't drift apart. */
export function atcBadgeKinds(positions: AtcPositionLite[]): string[] {
  return ["DEL", "GND", "TWR", "ATIS"].filter((k) => positions.some((p) => p.kind === k));
}

// Pixel dimensions mirroring the marker CSS in `markers/AtcMarkers.tsx`, used to size the hover
// hit-area below.
const BADGE_PX = 14; // AtcBadge: each pill's width/height
const BADGE_GAP_PX = 1; // AtcBadge: gap between stacked pills
const AREA_CHAR_PX = 7; // AreaPill: ui-monospace 11px 700-weight advance width, rounded up
const AREA_PAD_PX = 10; // AreaPill: `padding: "1px 5px"`, both sides

/** Half the visible marker's rendered width, in pixels — an airport's badge stack width for
 * `AtcBadge`, or the id pill's text width for `AreaPill`. */
function hoverRadiusPx(a: AtcAnchor): number {
  const halfWidth =
    a.type === "airport"
      ? (atcBadgeKinds(a.positions).length * (BADGE_PX + BADGE_GAP_PX)) / 2
      : (a.id.length * AREA_CHAR_PX + AREA_PAD_PX) / 2;
  return halfWidth + 4; // a small margin past the exact edge
}

/** An invisible pickable circle at each anchor so deck's getTooltip can fire on hover (DOM markers
 * sit under deck's event layer and can't be hovered directly). Sized per anchor to cover the actual
 * rendered marker — a fixed radius left most of a multi-badge stack or a long area id unpickable. */
export function buildAtcHoverLayer(anchors: AtcAnchor[]) {
  return new ScatterplotLayer<AtcAnchor>({
    id: "atc-hover",
    data: anchors,
    pickable: true,
    getPosition: (a) => [a.lon, a.lat],
    getRadius: hoverRadiusPx,
    radiusUnits: "pixels",
    radiusMinPixels: 13,
    getFillColor: [0, 0, 0, 0], // invisible, but still pickable
  });
}

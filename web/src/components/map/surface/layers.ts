import {PathLayer, PolygonLayer, ScatterplotLayer} from "@deck.gl/layers";
import {PathStyleExtension} from "@deck.gl/extensions";
import type {Layer} from "@deck.gl/core";

import type {
  AirportGate,
  AirportRampArea,
  AirportRunway,
  AirportSurface,
  AirportTaxiway,
} from "@/lib/airport-surface";

import {type MapPalette, readMapPalette} from "../lib/colors";
import {toDeckPath, type LatLng} from "../lib/geo";
import type {RGB, RGBA} from "../lib/types";

export type SurfaceKind = "gate" | "ramp" | "taxiway" | "runway";

export interface SelectedSurfaceItem {
  kind: SurfaceKind;
  id: string;
}

/** Each kind's colour, from the series / map tokens. */
export function surfaceColor(kind: SurfaceKind, palette: MapPalette = readMapPalette()): RGB {
  if (kind === "gate") return palette.series[2];
  if (kind === "taxiway") return palette.series[0];
  if (kind === "ramp") return palette.series[3];
  return palette.muted;
}

/** Fewest vertices each shape needs before it can be finalized/saved. */
export const MIN_SURFACE_POINTS: Record<SurfaceKind, number> = { gate: 1, taxiway: 3, ramp: 3, runway: 3 };

/** Whether a kind is a polygon (ramp/apron area, taxiway or runway pavement) rather than a point. */
export const isPolygonKind = (kind: SurfaceKind) => kind !== "gate";

/** A ramp area's or taxiway's rings as deck.gl `[lon, lat]` polygon rings. */
const toDeckRings = (rings: number[][][]) => rings.map((ring) => ring.map(([lat, lon]) => [lon, lat]));

/** A filled, geographic (so it scales with zoom) polygon layer — ramps, taxiways, runways share it. */
function polygonLayer<T extends { rings: number[][][] }>(
  id: string,
  data: T[],
  kind: SurfaceKind,
  palette: MapPalette,
): Layer {
  const [r, g, b] = surfaceColor(kind, palette);
  return new PolygonLayer<T>({
    id,
    data,
    pickable: true,
    getPolygon: (d) => toDeckRings(d.rings),
    stroked: true,
    filled: true,
    getLineColor: [r, g, b, 200] as RGBA,
    getFillColor: [r, g, b, 40] as RGBA,
    getLineWidth: 2,
    lineWidthUnits: "pixels",
    lineWidthMinPixels: 1,
  });
}

/**
 * Saved geometry: ramp/apron areas and taxiway/runway pavement as filled polygons, gates as points. The
 * item matching `selected` (if any) is left out — it's rendered instead by the draft layers below,
 * so a being-edited shape doesn't show twice.
 */
export function buildSurfaceLayers(
  surface: AirportSurface,
  selected: SelectedSurfaceItem | null,
  palette: MapPalette = readMapPalette(),
): Layer[] {
  const layers: Layer[] = [];
  const isSelected = (kind: SurfaceKind, id: string) => selected?.kind === kind && selected.id === id;

  // Largest pavement first: the later layer wins deck.gl picking, so the smaller shape stays on top
  // and stays clickable. FAA taxiway polygons overlap runways at most real airports (KORD 71 of
  // them, KDFW 72) and cross aprons, so ramps go down first, then runways, then taxiways (#278/#279).
  const ramps = surface.ramp_areas.filter((r) => !isSelected("ramp", r.id));
  if (ramps.length > 0) layers.push(polygonLayer<AirportRampArea>("surface-ramp-areas", ramps, "ramp", palette));

  const runways = surface.runways.filter((r) => !isSelected("runway", r.id));
  if (runways.length > 0) layers.push(polygonLayer<AirportRunway>("surface-runways", runways, "runway", palette));

  const taxiways = surface.taxiways.filter((t) => !isSelected("taxiway", t.id));
  if (taxiways.length > 0) layers.push(polygonLayer<AirportTaxiway>("surface-taxiways", taxiways, "taxiway", palette));

  const gates = surface.gates.filter((g) => !isSelected("gate", g.id));
  if (gates.length > 0) {
    const [r, g, b] = surfaceColor("gate", palette);
    layers.push(
      new ScatterplotLayer<AirportGate>({
        id: "surface-gates",
        data: gates,
        pickable: true,
        getPosition: (d) => [d.lon, d.lat],
        getFillColor: [r, g, b, 230] as RGBA,
        stroked: true,
        getLineColor: [...palette.ink, 220] as RGBA,
        getLineWidth: 1,
        lineWidthUnits: "pixels",
        // #517: a non-gate stand draws smaller, so a GA field's tie-downs are distinguishable from its
        // terminal gates at a glance and not only on hover. Size rather than colour — DESIGN.md allows
        // one accent. A null `kind` (every manual/osm/crc row) keeps the original radius, so nothing
        // already on an operator's screen moves.
        getRadius: (d) => (d.kind && d.kind !== "gate" ? 3 : 5),
        radiusUnits: "pixels",
        radiusMinPixels: 3,
      }),
    );
  }

  return layers;
}

/**
 * How many drag handles a selected shape may draw at once.
 *
 * Measured against the real dataset (`backend/data/faa_surface.json`, 24,583 rings) rather than
 * guessed: the median ring is 15 vertices and the 99th percentile is 110, so only ~1.2% of shapes
 * exceed 100 at all. A cap below that percentile leaves 98.8% of shapes byte-identical while fixing
 * the tail — KDCA's main ramp is 352 vertices and the worst in the set (KSFB) is 1,920, which at one
 * handle per vertex merge into a bead chain that hides the boundary they exist to let you edit
 * (#538).
 *
 * 80 rather than 110 because the goal is a shape a human can work with, not merely a smaller number:
 * it is a 4.4x reduction for KDCA and still more handles than anyone grabs in a session.
 */
const MAX_HANDLES = 80;

/** One drag handle: where it sits, and which vertex of the *original* path it stands for. */
export type VertexHandle = { pos: [number, number]; index: number };

/**
 * The handles to draw for `path`, capped at [`MAX_HANDLES`].
 *
 * Returns the whole path, in order, when it is already short enough — the common case by a wide
 * margin, and one that must come back unchanged.
 *
 * # `index` is load-bearing, not decoration
 *
 * `SurfaceMap`'s vertex drag used to read deck's `info.index` — the position within the *layer's*
 * data — and assign straight into `draft.points` at that index. That was only correct while the two
 * arrays were 1:1. Decimating without carrying the original index would mean dragging a handle moved
 * a different vertex of the ring, silently and invisibly: the shape would deform somewhere the user
 * was not looking. So each handle names its own vertex, and the drag reads that.
 *
 * # Sampling
 *
 * Evenly spaced, with the first and last vertex always kept so a closed ring still reads as closed
 * and the shape's extent does not appear to shrink. A ring whose detail is finer than the sample
 * step loses handles, not geometry: `draft.points` is untouched, so the stored shape is exactly what
 * was imported and the polygon renders from the full ring as before.
 */
export function handleVertices(path: [number, number][]): VertexHandle[] {
  if (path.length <= MAX_HANDLES) return path.map((pos, index) => ({ pos, index }));

  // Spaced so the first and last samples land exactly on the first and last vertex: with
  // `step = (len-1)/(MAX-1)`, the final iteration rounds to `len-1` for every length above the cap
  // (checked for 81..5000). An explicit clamp for the last index was here and was unreachable, so
  // it is gone rather than left as a guard no test could reach.
  const step = (path.length - 1) / (MAX_HANDLES - 1);
  const handles: VertexHandle[] = [];
  for (let i = 0; i < MAX_HANDLES; i += 1) {
    const index = Math.round(i * step);
    // `step > 1` whenever the cap applies, so this cannot repeat — kept because a future change to
    // the cap or the spacing could make it possible, and a duplicated handle is invisible on screen
    // but makes two handles fight for the same pick.
    if (handles.length && handles[handles.length - 1].index === index) continue;
    handles.push({ pos: path[index], index });
  }
  return handles;
}

/**
 * The in-progress draft: a point (gate), an open dashed polyline (a ramp/apron area or taxiway
 * still being drawn), or a filled ring (once closed) — plus a draggable handle per vertex,
 * mirroring `layers/draft.ts`'s FCA draft rendering extended to three shape kinds. `points` never
 * carries a polygon's closing duplicate — the ring is closed visually here only.
 */
export function buildSurfaceDraftLayers(
  kind: SurfaceKind,
  points: LatLng[],
  phase: "draw" | "edit",
  palette: MapPalette = readMapPalette(),
): Layer[] {
  const [r, g, b] = surfaceColor(kind, palette);
  const path = toDeckPath(points);
  const layers: Layer[] = [];
  const closedRing = isPolygonKind(kind) && phase === "edit" && path.length >= 3;

  if (closedRing) {
    layers.push(
      new PolygonLayer<{ contour: [number, number][] }>({
        id: "surface-draft-polygon",
        data: [{ contour: path }],
        getPolygon: (d) => d.contour,
        stroked: true,
        filled: true,
        getLineColor: [r, g, b, 230] as RGBA,
        getFillColor: [r, g, b, 60] as RGBA,
        getLineWidth: 3,
        lineWidthUnits: "pixels",
      }),
    );
  } else if (path.length >= 2) {
    layers.push(
      new PathLayer<{ path: [number, number][] }>({
        id: "surface-draft-line",
        data: [{ path }],
        getPath: (d) => d.path,
        getColor: [r, g, b, 255] as RGBA,
        getWidth: 4,
        widthUnits: "pixels",
        widthMinPixels: 3,
        extensions: [new PathStyleExtension({ dash: true })],
        ...({ getDashArray: [6, 6], dashJustified: true } as Record<string, unknown>),
      }),
    );
  }

  layers.push(
    new ScatterplotLayer<VertexHandle>({
      id: "surface-draft-vertices",
      // Capped, and each handle carries the index of the vertex it represents — see
      // `handleVertices`. The drag in `SurfaceMap` reads that index, not deck's `info.index`.
      data: handleVertices(path),
      pickable: true,
      getPosition: (d) => d.pos,
      getFillColor: kind === "gate" ? ([r, g, b, 255] as RGBA) : ([...palette.ink, 255] as RGBA),
      stroked: true,
      getLineColor: [r, g, b, 255] as RGBA,
      getLineWidth: 2,
      lineWidthUnits: "pixels",
      lineWidthMinPixels: 2,
      getRadius: kind === "gate" ? 8 : 6,
      radiusUnits: "pixels",
      radiusMinPixels: kind === "gate" ? 7 : 5,
    }),
  );

  return layers;
}

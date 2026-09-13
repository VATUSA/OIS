import {PathLayer, PolygonLayer, ScatterplotLayer} from "@deck.gl/layers";
import {PathStyleExtension} from "@deck.gl/extensions";
import type {Layer} from "@deck.gl/core";

import type {AirportGate, AirportRampArea, AirportSurface, AirportTaxiway} from "@/lib/airport-surface";

import {hexToRgb} from "../lib/colors";
import {toDeckPath, type LatLng} from "../lib/geo";
import type {RGBA} from "../lib/types";

export type SurfaceKind = "gate" | "ramp" | "taxiway";

export interface SelectedSurfaceItem {
  kind: SurfaceKind;
  id: string;
}

export const SURFACE_COLORS: Record<SurfaceKind, string> = {
  gate: "#f59e0b",
  taxiway: "#38bdf8",
  ramp: "#a78bfa",
};

/** Fewest vertices each shape needs before it can be finalized/saved. */
export const MIN_SURFACE_POINTS: Record<SurfaceKind, number> = { gate: 1, taxiway: 2, ramp: 3 };

/**
 * Saved geometry: ramp/apron areas as filled polygons, taxiways as lines, gates as points. The
 * item matching `selected` (if any) is left out — it's rendered instead by the draft layers below,
 * so a being-edited shape doesn't show twice.
 */
export function buildSurfaceLayers(surface: AirportSurface, selected: SelectedSurfaceItem | null): Layer[] {
  const layers: Layer[] = [];
  const isSelected = (kind: SurfaceKind, id: string) => selected?.kind === kind && selected.id === id;

  const ramps = surface.ramp_areas.filter((r) => !isSelected("ramp", r.id));
  if (ramps.length > 0) {
    const [r, g, b] = hexToRgb(SURFACE_COLORS.ramp);
    layers.push(
      new PolygonLayer<AirportRampArea>({
        id: "surface-ramp-areas",
        data: ramps,
        pickable: true,
        getPolygon: (d) => d.rings.map((ring) => ring.map(([lat, lon]) => [lon, lat])),
        stroked: true,
        filled: true,
        getLineColor: [r, g, b, 200] as RGBA,
        getFillColor: [r, g, b, 40] as RGBA,
        getLineWidth: 2,
        lineWidthUnits: "pixels",
        lineWidthMinPixels: 1,
      }),
    );
  }

  const taxiways = surface.taxiways.filter((t) => !isSelected("taxiway", t.id));
  if (taxiways.length > 0) {
    const [r, g, b] = hexToRgb(SURFACE_COLORS.taxiway);
    layers.push(
      new PathLayer<AirportTaxiway>({
        id: "surface-taxiways",
        data: taxiways,
        pickable: true,
        getPath: (d) => toDeckPath(d.points as LatLng[]),
        getColor: [r, g, b, 220] as RGBA,
        getWidth: 3,
        widthUnits: "pixels",
        widthMinPixels: 2,
      }),
    );
  }

  const gates = surface.gates.filter((g) => !isSelected("gate", g.id));
  if (gates.length > 0) {
    const [r, g, b] = hexToRgb(SURFACE_COLORS.gate);
    layers.push(
      new ScatterplotLayer<AirportGate>({
        id: "surface-gates",
        data: gates,
        pickable: true,
        getPosition: (d) => [d.lon, d.lat],
        getFillColor: [r, g, b, 230] as RGBA,
        stroked: true,
        getLineColor: [255, 255, 255, 220] as RGBA,
        getLineWidth: 1,
        lineWidthUnits: "pixels",
        getRadius: 5,
        radiusUnits: "pixels",
        radiusMinPixels: 4,
      }),
    );
  }

  return layers;
}

/**
 * The in-progress draft: a point (gate), an open dashed polyline (taxiway, or a ramp/apron area
 * still being drawn), or a filled ring (a ramp/apron area once closed) — plus a draggable handle
 * per vertex, mirroring `layers/draft.ts`'s FCA draft rendering extended to three shape kinds.
 * `points` never carries a ramp's closing duplicate — the ring is closed visually here only.
 */
export function buildSurfaceDraftLayers(kind: SurfaceKind, points: LatLng[], phase: "draw" | "edit"): Layer[] {
  const [r, g, b] = hexToRgb(SURFACE_COLORS[kind]);
  const path = toDeckPath(points);
  const layers: Layer[] = [];
  const closedRing = kind === "ramp" && phase === "edit" && path.length >= 3;

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
    new ScatterplotLayer<{ pos: [number, number] }>({
      id: "surface-draft-vertices",
      data: path.map((pos) => ({ pos })),
      pickable: true,
      getPosition: (d) => d.pos,
      getFillColor: kind === "gate" ? ([r, g, b, 255] as RGBA) : [255, 255, 255, 255],
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

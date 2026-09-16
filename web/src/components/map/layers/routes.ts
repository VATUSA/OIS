import {PathLayer, ScatterplotLayer, TextLayer} from "@deck.gl/layers";
import type {Layer} from "@deck.gl/core";

import {hexToRgb, type MapPalette, readMapPalette} from "../lib/colors";
import {midpointOf, toDeckPath, type LatLng} from "../lib/geo";
import type {RGB, RGBA} from "../lib/types";

/** A saved named route (filed-route string resolved to anchors). */
export interface NamedRoute {
  id: string;
  name: string;
  color: string;
  points: number[][]; // [lat, lon] pairs
  waypoints: { name: string; lat: number; lon: number }[];
}

/** Saved routes as solid per-color polylines with endpoint dots, name labels, and (toggled) fixes. */
export function buildNamedRouteLayers(
  routes: NamedRoute[],
  selectedRouteId: string | null | undefined,
  labeled: Set<string>,
  palette: MapPalette = readMapPalette(),
): Layer[] {
  const halo = [...palette.halo, 220] as RGBA;
  const valid = routes.filter((r) => r.points.length >= 2);
  if (valid.length === 0) return [];

  const lineData = valid.map((r) => {
    const rgb = hexToRgb(r.color);
    const selected = r.id === selectedRouteId;
    return { path: toDeckPath(r.points as LatLng[]), rgb, selected };
  });

  const lines = new PathLayer<(typeof lineData)[number]>({
    id: "named-routes",
    data: lineData,
    getPath: (d) => d.path,
    getColor: (d) => [...d.rgb, d.selected ? 255 : 216] as RGBA,
    getWidth: (d) => (d.selected ? 5 : 3),
    widthUnits: "pixels",
    widthMinPixels: 2,
    capRounded: true,
    jointRounded: true,
    updateTriggers: { getColor: [selectedRouteId], getWidth: [selectedRouteId] },
  });

  const endpoints = new ScatterplotLayer<{ pos: [number, number]; rgb: RGB }>({
    id: "named-route-endpoints",
    data: lineData.flatMap((d) => [
      { pos: d.path[0], rgb: d.rgb },
      { pos: d.path[d.path.length - 1], rgb: d.rgb },
    ]),
    getPosition: (d) => d.pos,
    getFillColor: (d) => [...d.rgb, 230] as RGBA,
    getRadius: 4,
    radiusUnits: "pixels",
    radiusMinPixels: 3,
  });

  const nameLabels = new TextLayer<{ pos: [number, number]; name: string; rgb: RGB }>({
    id: "named-route-labels",
    data: valid.map((r) => {
      const [lat, lon] = midpointOf(r.points as LatLng[]);
      return { pos: [lon, lat] as [number, number], name: r.name, rgb: hexToRgb(r.color) };
    }),
    getPosition: (d) => d.pos,
    getText: (d) => d.name,
    getColor: (d) => [...d.rgb, 255] as RGBA,
    getSize: 12,
    getPixelOffset: [0, -12],
    getTextAnchor: "middle",
    getAlignmentBaseline: "bottom",
    fontWeight: 700,
    outlineWidth: 2,
    outlineColor: halo,
    fontSettings: { sdf: true },
  });

  const layers: Layer[] = [lines, endpoints, nameLabels];

  // Per-route fix dots + names (toggled).
  const fixRoutes = valid.filter((r) => labeled.has(r.id));
  if (fixRoutes.length > 0) {
    const fixes = fixRoutes.flatMap((r) =>
      r.waypoints.map((w) => ({ pos: [w.lon, w.lat] as [number, number], name: w.name, rgb: hexToRgb(r.color) })),
    );
    layers.push(
      new ScatterplotLayer<(typeof fixes)[number]>({
        id: "named-route-fix-dots",
        data: fixes,
        getPosition: (d) => d.pos,
        getFillColor: (d) => [...d.rgb, 255] as RGBA,
        getRadius: 2.5,
        radiusUnits: "pixels",
        radiusMinPixels: 2,
        updateTriggers: { data: [labeled] },
      }),
      new TextLayer<(typeof fixes)[number]>({
        id: "named-route-fix-labels",
        data: fixes,
        getPosition: (d) => d.pos,
        getText: (d) => d.name,
        getColor: (d) => [...d.rgb, 255] as RGBA,
        getSize: 10,
        getPixelOffset: [6, -2],
        getTextAnchor: "start",
        getAlignmentBaseline: "center",
        fontWeight: 600,
        outlineWidth: 2,
        outlineColor: halo,
        fontSettings: { sdf: true },
      }),
    );
  }

  return layers;
}

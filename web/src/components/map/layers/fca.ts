import {PathLayer, ScatterplotLayer, TextLayer} from "@deck.gl/layers";
import {PathStyleExtension} from "@deck.gl/extensions";

import {hexToRgb} from "../lib/colors";
import {midpointOf, toDeckPath, type LatLng} from "../lib/geo";
import type {RGBA} from "../lib/types";

/** The subset of an FCA the map needs to render its line (open dashed polyline, selectable). */
export interface MapFca {
  id: string;
  name: string;
  color: string;
  enabled: boolean;
  points: LatLng[]; // [lat, lon] pairs
}

interface FcaDatum {
  id: string;
  path: [number, number][];
  color: RGBA;
  selected: boolean;
}

/**
 * Saved FCAs as dashed, clickable polylines (not closed polygons) tinted their own color, plus
 * endpoint dots. The midpoint name label is an HTML marker (see FcaLabelMarker), not drawn here.
 */
export function buildFcaLayers(fcas: MapFca[], selectedId: string | null | undefined) {
  const data: FcaDatum[] = fcas
    .filter((f) => f.points.length >= 2)
    .map((f) => {
      const [r, g, b] = hexToRgb(f.color);
      const selected = f.id === selectedId;
      const a = selected ? 255 : f.enabled ? 220 : 110;
      return { id: f.id, path: toDeckPath(f.points), color: [r, g, b, a] as RGBA, selected };
    });

  // PathStyleExtension adds getDashArray/dashJustified, which aren't in the base PathLayer prop type;
  // spread them (a spread bypasses the excess-property check, unlike an inline literal).
  const dashProps = { getDashArray: [4, 8], dashJustified: true } as Record<string, unknown>;
  const line = new PathLayer<FcaDatum>({
    id: "fca-lines",
    data,
    pickable: true,
    getPath: (d) => d.path,
    getColor: (d) => d.color,
    getWidth: (d) => (d.selected ? 5 : 3),
    widthUnits: "pixels",
    widthMinPixels: 4, // a comfortable pick target for a thin line
    capRounded: true,
    jointRounded: true,
    extensions: [new PathStyleExtension({ dash: true })],
    ...dashProps,
    updateTriggers: { getColor: [selectedId], getWidth: [selectedId] },
  });

  const endpoints = new ScatterplotLayer<{ pos: [number, number]; color: RGBA }>({
    id: "fca-endpoints",
    data: data.flatMap((d) => [
      { pos: d.path[0], color: d.color },
      { pos: d.path[d.path.length - 1], color: d.color },
    ]),
    getPosition: (d) => d.pos,
    getFillColor: (d) => d.color,
    getRadius: 3.5,
    radiusUnits: "pixels",
    radiusMinPixels: 3,
    updateTriggers: { getFillColor: [selectedId] },
  });

  const labels = new TextLayer<{ pos: [number, number]; name: string; color: RGBA }>({
    id: "fca-labels",
    data: fcas
      .filter((f) => f.points.length >= 2)
      .map((f) => {
        const [lat, lon] = midpointOf(f.points);
        const [r, g, b] = hexToRgb(f.color);
        return { pos: [lon, lat] as [number, number], name: f.name, color: [r, g, b, 255] as RGBA };
      }),
    getPosition: (d) => d.pos,
    getText: (d) => d.name,
    getColor: (d) => d.color,
    getSize: 12,
    getPixelOffset: [0, -12],
    getTextAnchor: "middle",
    getAlignmentBaseline: "bottom",
    fontWeight: 700,
    outlineWidth: 2,
    outlineColor: [0, 0, 0, 230],
    fontSettings: { sdf: true },
  });

  return [line, endpoints, labels];
}

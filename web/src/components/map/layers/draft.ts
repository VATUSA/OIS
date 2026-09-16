import {PathLayer, ScatterplotLayer} from "@deck.gl/layers";
import {PathStyleExtension} from "@deck.gl/extensions";
import type {Layer} from "@deck.gl/core";

import {hexToRgb, type MapPalette, readMapPalette} from "../lib/colors";
import {toDeckPath, type LatLng} from "../lib/geo";
import type {RGBA} from "../lib/types";

/** The in-progress FCA line being drawn/edited. */
export interface DraftLine {
  color: string;
  points: LatLng[]; // [lat, lon] pairs
}

/**
 * The working draft: a dashed polyline (≥2 pts) plus a draggable handle per vertex. The handles are a
 * pickable ScatterplotLayer (id "draft-vertices") so deck picking can identify which vertex a drag
 * grabbed (info.index).
 */
export function buildDraftLayers(draft: DraftLine, palette: MapPalette = readMapPalette()): Layer[] {
  const [r, g, b] = hexToRgb(draft.color);
  const path = toDeckPath(draft.points);
  const layers: Layer[] = [];

  if (path.length >= 2) {
    layers.push(
      new PathLayer<{ path: [number, number][] }>({
        id: "draft-line",
        data: [{ path }],
        getPath: (d) => d.path,
        getColor: [r, g, b, 255] as RGBA,
        getWidth: 4,
        widthUnits: "pixels",
        widthMinPixels: 3,
        extensions: [new PathStyleExtension({ dash: true })],
        ...({ getDashArray: [6, 6], dashJustified: true } as Record<string, unknown>),
        updateTriggers: { getColor: [draft.color] },
      }),
    );
  }

  layers.push(
    new ScatterplotLayer<{ pos: [number, number] }>({
      id: "draft-vertices",
      data: path.map((pos) => ({ pos })),
      pickable: true,
      getPosition: (d) => d.pos,
      getFillColor: [...palette.ink, 255] as RGBA,
      stroked: true,
      getLineColor: [r, g, b, 255] as RGBA,
      getLineWidth: 2,
      lineWidthUnits: "pixels",
      lineWidthMinPixels: 2,
      getRadius: 6,
      radiusUnits: "pixels",
      radiusMinPixels: 5,
      updateTriggers: { getLineColor: [draft.color], getFillColor: [palette] },
    }),
  );

  return layers;
}

import {GeoJsonLayer} from "@deck.gl/layers";

import {type MapPalette, readMapPalette} from "../lib/colors";
import type {RGBA} from "../lib/types";

/**
 * ARTCC boundary outlines (non-interactive). `emphasis` draws a bolder line + faint fill — used by the
 * facility map, which shows a single facility's airspace and wants it to read clearly.
 */
export function buildBoundaryLayer(
  data: GeoJSON.FeatureCollection,
  palette: MapPalette = readMapPalette(),
  emphasis = false,
) {
  const [r, g, b] = palette.boundary;
  // Faint interior fill for an emphasized (single-facility) boundary.
  const fill: RGBA = [r, g, b, 16];
  return new GeoJsonLayer({
    id: "artcc-boundaries",
    data,
    stroked: true,
    filled: emphasis,
    getFillColor: emphasis ? fill : [r, g, b, 0],
    getLineColor: palette.boundary,
    getLineWidth: emphasis ? 2 : 1,
    lineWidthUnits: "pixels",
    lineWidthMinPixels: emphasis ? 2 : 1,
    updateTriggers: {
      getLineColor: [palette],
      getLineWidth: [emphasis],
      getFillColor: [palette, emphasis],
    },
  });
}

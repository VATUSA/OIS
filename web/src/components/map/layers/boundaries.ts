import {GeoJsonLayer} from "@deck.gl/layers";

import type {Theme} from "../lib/constants";
import {boundaryColor} from "../lib/colors";

/** Faint interior fill for an emphasized (single-facility) boundary. */
const boundaryFill = (theme: Theme): [number, number, number, number] =>
  theme === "dark" ? [120, 140, 170, 18] : [80, 100, 140, 14];

/**
 * ARTCC boundary outlines (non-interactive). `emphasis` draws a bolder line + faint fill — used by the
 * facility map, which shows a single facility's airspace and wants it to read clearly.
 */
export function buildBoundaryLayer(
  data: GeoJSON.FeatureCollection,
  theme: Theme,
  emphasis = false,
) {
  return new GeoJsonLayer({
    id: "artcc-boundaries",
    data,
    stroked: true,
    filled: emphasis,
    getFillColor: emphasis ? boundaryFill(theme) : [0, 0, 0, 0],
    getLineColor: boundaryColor(theme),
    getLineWidth: emphasis ? 2 : 1,
    lineWidthUnits: "pixels",
    lineWidthMinPixels: emphasis ? 2 : 1,
    updateTriggers: {
      getLineColor: [theme],
      getLineWidth: [emphasis],
      getFillColor: [theme, emphasis],
    },
  });
}

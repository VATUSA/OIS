import {GeoJsonLayer} from "@deck.gl/layers";

import type {Theme} from "../lib/constants";
import {boundaryColor} from "../lib/colors";

/** ARTCC boundary outlines (non-interactive). */
export function buildBoundaryLayer(data: GeoJSON.FeatureCollection, theme: Theme) {
  return new GeoJsonLayer({
    id: "artcc-boundaries",
    data,
    stroked: true,
    filled: false,
    getLineColor: boundaryColor(theme),
    getLineWidth: 1,
    lineWidthUnits: "pixels",
    lineWidthMinPixels: 1,
    updateTriggers: { getLineColor: [theme] },
  });
}

import {PathLayer, ScatterplotLayer, TextLayer} from "@deck.gl/layers";

import {type MapPalette, readMapPalette} from "../lib/colors";
import type {NormAircraft, PathDatum, RGBA} from "../lib/types";

type Waypoint = { name: string; lat: number; lon: number };

/** Faint flown-trail polylines behind every shown aircraft. */
export function buildTrailLayer(data: PathDatum[], palette: MapPalette = readMapPalette()) {
  const c = palette.aircraft;
  return new PathLayer<PathDatum>({
    id: "all-trails",
    data,
    getPath: (d) => d.path,
    getColor: [...c, 80] as RGBA,
    getWidth: 1.4,
    widthUnits: "pixels",
    widthMinPixels: 1,
    updateTriggers: { getColor: [palette] },
  });
}

/** Filed-route polylines for every shown flight (violet, faint). */
export function buildRouteOverlayLayer(data: PathDatum[], palette: MapPalette = readMapPalette()) {
  return new PathLayer<PathDatum>({
    id: "routes-all",
    data,
    getPath: (d) => d.path,
    getColor: [...palette.route, 90] as RGBA,
    getWidth: 1.2,
    widthUnits: "pixels",
    widthMinPixels: 1,
  });
}

/** Range rings around each shown aircraft (radius in NM). */
export function buildRingLayer(data: NormAircraft[], nm: number, palette: MapPalette = readMapPalette()) {
  const c = palette.aircraft;
  return new ScatterplotLayer<NormAircraft>({
    id: "range-rings",
    data,
    getPosition: (d) => [d.lon, d.lat],
    getRadius: nm * 1852, // NM → metres
    radiusUnits: "meters",
    stroked: true,
    filled: false,
    getLineColor: [...c, 150] as RGBA,
    lineWidthUnits: "pixels",
    getLineWidth: 1.1,
    lineWidthMinPixels: 1,
    updateTriggers: { getRadius: [nm], getLineColor: [palette] },
  });
}

/** The selected flight's flown-so-far history trail (highlighted). */
export function buildSelectedTrackLayer(data: PathDatum[], palette: MapPalette = readMapPalette()) {
  return new PathLayer<PathDatum>({
    id: "selected-track",
    data,
    getPath: (d) => d.path,
    getColor: [...palette.highlight, 220] as RGBA,
    getWidth: 2,
    widthUnits: "pixels",
    widthMinPixels: 2,
    capRounded: true,
    jointRounded: true,
  });
}

/** The selected/plotted flight's filed route (violet). */
export function buildSelectedRouteLayer(data: PathDatum[], palette: MapPalette = readMapPalette()) {
  return new PathLayer<PathDatum>({
    id: "selected-route",
    data,
    getPath: (d) => d.path,
    getColor: [...palette.route, 230] as RGBA,
    getWidth: 2,
    widthUnits: "pixels",
    widthMinPixels: 2,
    capRounded: true,
    jointRounded: true,
  });
}

/** Named-waypoint labels along the plotted filed route. */
export function buildWaypointLayer(data: Waypoint[], palette: MapPalette = readMapPalette()) {
  return new TextLayer<Waypoint>({
    id: "selected-route-waypoints",
    data,
    getPosition: (d) => [d.lon, d.lat],
    getText: (d) => d.name,
    getColor: [...palette.route, 255] as RGBA,
    getSize: 10,
    getPixelOffset: [0, -10],
    getTextAnchor: "middle",
    getAlignmentBaseline: "bottom",
    background: true,
    getBackgroundColor: palette.waypointBg,
    backgroundPadding: [2, 1],
    updateTriggers: { getColor: [palette], getBackgroundColor: [palette] },
  });
}

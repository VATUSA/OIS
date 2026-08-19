import {IconLayer, PathLayer, ScatterplotLayer, TextLayer} from "@deck.gl/layers";
import type {Layer} from "@deck.gl/core";

import {aircraftIconUrl} from "@/lib/aircraft-icons";
import {hexToRgb} from "../lib/colors";
import {TRIANGLE_ICON} from "../lib/icons";
import type {RGBA} from "../lib/types";

/** A crossing ("matched") flight for the selected FCA. */
export interface MatchedFlight {
  callsign: string;
  aircraft_type: string;
  seq: number;
  heading: number;
  lat: number;
  lon: number;
  cross_lat: number;
  cross_lon: number;
  path: number[][]; // [lat, lon] pairs (remaining route)
  dep: string;
  arr: string;
  altitude: number;
  groundspeed: number;
  distance_nm: number;
}

/**
 * Matched (crossing) traffic for the selected FCA, tinted the FCA color and numbered by crossing
 * sequence: a faint trail to the crossing, a white crossing-point dot, the plane glyph, and a seq
 * badge. The glyph is pickable so clicking it plots the route (layer id "matched").
 */
export function buildMatchedLayers(
  matched: MatchedFlight[],
  colorHex: string,
  style: "silhouette" | "triangle",
): Layer[] {
  const [r, g, b] = hexToRgb(colorHex);
  const tint: RGBA = [r, g, b, 255];
  const withPos = matched.filter((f) => f.lat !== 0 || f.lon !== 0);

  const trails = new PathLayer<MatchedFlight>({
    id: "matched-trails",
    data: withPos,
    getPath: (f): [number, number][] =>
      f.path && f.path.length >= 2
        ? f.path.map(([lat, lon]) => [lon, lat] as [number, number])
        : [
            [f.lon, f.lat],
            [f.cross_lon, f.cross_lat],
          ],
    getColor: [r, g, b, 140] as RGBA,
    getWidth: 1.5,
    widthUnits: "pixels",
    widthMinPixels: 1,
    updateTriggers: { getColor: [colorHex], getPath: [] },
  });

  const crossDots = new ScatterplotLayer<MatchedFlight>({
    id: "matched-cross",
    data: matched,
    getPosition: (f) => [f.cross_lon, f.cross_lat],
    getFillColor: [255, 255, 255, 230],
    getRadius: 3,
    radiusUnits: "pixels",
    radiusMinPixels: 2,
  });

  const glyphs = new IconLayer<MatchedFlight>({
    id: "matched",
    data: withPos,
    pickable: true,
    getIcon: (f) => {
      if (style === "triangle") return { id: "tri", url: TRIANGLE_ICON, width: 24, height: 24, mask: true };
      const url = aircraftIconUrl(f.aircraft_type);
      return { id: url, url, width: 48, height: 48, mask: true };
    },
    getPosition: (f) => [f.lon, f.lat],
    getAngle: (f) => 360 - f.heading,
    getColor: tint,
    getSize: style === "triangle" ? 14 : 22,
    sizeUnits: "pixels",
    billboard: false,
    updateTriggers: { getColor: [colorHex], getIcon: [style] },
  });

  const badges = new TextLayer<MatchedFlight>({
    id: "matched-seq",
    data: withPos,
    getPosition: (f) => [f.lon, f.lat],
    getText: (f) => String(f.seq),
    getColor: [10, 10, 10, 255],
    getSize: 10,
    getPixelOffset: [11, -8],
    getTextAnchor: "middle",
    getAlignmentBaseline: "center",
    background: true,
    getBackgroundColor: tint,
    backgroundPadding: [3, 1],
    fontWeight: 700,
    updateTriggers: { getBackgroundColor: [colorHex] },
  });

  return [trails, crossDots, glyphs, badges];
}

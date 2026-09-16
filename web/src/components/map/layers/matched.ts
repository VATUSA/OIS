import {IconLayer, PathLayer, ScatterplotLayer, TextLayer} from "@deck.gl/layers";
import type {Layer} from "@deck.gl/core";

import {aircraftIconUrl} from "@/lib/aircraft-icons";
import {aircraftTypeScale} from "@/lib/aircraft-icon-size";
import {clampGlyphSize} from "./aircraft";
import {hexToRgb, type MapPalette, readMapPalette} from "../lib/colors";
import {toDeckPath, type LatLng} from "../lib/geo";
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
 * sequence: a faint trail to the crossing, a contrasting crossing-point dot, the plane glyph, and a seq
 * badge. The glyph is pickable so clicking it plots the route (layer id "matched").
 */
export function buildMatchedLayers(
  matched: MatchedFlight[],
  colorHex: string,
  style: "silhouette" | "triangle",
  sizeScale = 1,
  // Suffix appended to every layer id so several FCAs' matched traffic can coexist (overview mode).
  // The pickable glyph layer's id always starts with "matched" (see the click handler in TrafficMap).
  keySuffix = "",
  palette: MapPalette = readMapPalette(),
): Layer[] {
  const [r, g, b] = hexToRgb(colorHex);
  const tint: RGBA = [r, g, b, 255];
  const withPos = matched.filter((f) => f.lat !== 0 || f.lon !== 0);

  const trails = new PathLayer<MatchedFlight>({
    id: `matched-trails${keySuffix}`,
    data: withPos,
    getPath: (f): [number, number][] =>
      f.path && f.path.length >= 2
        ? toDeckPath(f.path as LatLng[])
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
    id: `matched-cross${keySuffix}`,
    data: matched,
    getPosition: (f) => [f.cross_lon, f.cross_lat],
    getFillColor: [...palette.ink, 230] as RGBA,
    getRadius: 3,
    radiusUnits: "pixels",
    radiusMinPixels: 2,
    updateTriggers: { getFillColor: [palette] },
  });

  const glyphs = new IconLayer<MatchedFlight>({
    id: `matched${keySuffix}`,
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
    getSize: (f) =>
      clampGlyphSize(
        (style === "triangle" ? 14 : 22) * sizeScale * (style === "triangle" ? 1 : aircraftTypeScale(f.aircraft_type)),
      ),
    sizeUnits: "pixels",
    billboard: false,
    updateTriggers: { getColor: [colorHex], getIcon: [style], getSize: [style, sizeScale] },
  });

  const badges = new TextLayer<MatchedFlight>({
    id: `matched-seq${keySuffix}`,
    data: withPos,
    getPosition: (f) => [f.lon, f.lat],
    getText: (f) => String(f.seq),
    getColor: [...palette.halo, 255] as RGBA,
    getSize: 10,
    getPixelOffset: [11, -8],
    getTextAnchor: "middle",
    getAlignmentBaseline: "center",
    background: true,
    getBackgroundColor: tint,
    backgroundPadding: [3, 1],
    fontWeight: 700,
    updateTriggers: { getColor: [palette], getBackgroundColor: [colorHex] },
  });

  return [trails, crossDots, glyphs, badges];
}

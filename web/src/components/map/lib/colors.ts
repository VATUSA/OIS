import {useMemo} from "react";
import {parseColor, readToken, useTokens} from "@ois/ui";

import type {Theme} from "./constants";
import type {RGB, RGBA} from "./types";

/**
 * Map colours come from the `--map-*`, `--atc-*` and `--series-*` design tokens (globals.css), never
 * from literals here. Components read them with `useMapPalette()`, which re-reads on a theme switch;
 * the plain functions below read the tokens at call time for non-React callers.
 */

const SERIES = [1, 2, 3, 4, 5, 6, 7, 8] as const;

const MAP_TOKENS = [
  "map-aircraft",
  "map-highlight",
  "map-route",
  "map-boundary",
  "map-label",
  "map-label-bg",
  "map-waypoint-bg",
  "map-apron",
  "map-taxiway",
  "map-runway",
  "atc-del",
  "atc-gnd",
  "atc-twr",
  "atc-app",
  "atc-ctr",
  "atc-atis",
  "ground",
  "ink",
  "ink-3",
  "series-1",
  "series-2",
  "series-3",
  "series-4",
  "series-5",
  "series-6",
  "series-7",
  "series-8",
] as const;

type MapToken = (typeof MAP_TOKENS)[number];

export interface MapPalette {
  aircraft: RGB;
  highlight: RGB;
  route: RGB;
  boundary: RGBA;
  label: RGB;
  labelBg: RGBA;
  waypointBg: RGBA;
  /** Aeroway overlay colours as CSS strings (MapLibre paint). */
  apron: string;
  taxiway: string;
  runway: string;
  /** ATC position hue, keyed by kind (DEL/GND/TWR/APP/CTR/ATIS). */
  atc: Record<string, RGB>;
  /** Contrast halo behind map text and on markers (the page ground). */
  halo: RGB;
  /** Strong contrast fill (vertex handles, crossing dots). */
  ink: RGB;
  /** Neutral fallback for unknown data colours. */
  muted: RGB;
  /** `--series-1..8`, resolved. */
  series: RGB[];
}

const toRgb = (v: string): RGB => parseColor(v).slice(0, 3) as RGB;

function paletteFrom(v: Record<MapToken, string>): MapPalette {
  return {
    aircraft: toRgb(v["map-aircraft"]),
    highlight: toRgb(v["map-highlight"]),
    route: toRgb(v["map-route"]),
    boundary: parseColor(v["map-boundary"]),
    label: toRgb(v["map-label"]),
    labelBg: parseColor(v["map-label-bg"]),
    waypointBg: parseColor(v["map-waypoint-bg"]),
    apron: v["map-apron"],
    taxiway: v["map-taxiway"],
    runway: v["map-runway"],
    atc: {
      DEL: toRgb(v["atc-del"]),
      GND: toRgb(v["atc-gnd"]),
      TWR: toRgb(v["atc-twr"]),
      APP: toRgb(v["atc-app"]),
      CTR: toRgb(v["atc-ctr"]),
      ATIS: toRgb(v["atc-atis"]),
    },
    halo: toRgb(v.ground),
    ink: toRgb(v.ink),
    muted: toRgb(v["ink-3"]),
    series: SERIES.map((n) => toRgb(v[`series-${n}`])),
  };
}

/** The map palette for the current theme, read now (non-React callers; outside a browser, greys). */
export function readMapPalette(): MapPalette {
  return paletteFrom(Object.fromEntries(MAP_TOKENS.map((n) => [n, readToken(n)])) as Record<MapToken, string>);
}

/** The map palette, re-read whenever the theme changes. */
export function useMapPalette(): MapPalette {
  const values = useTokens(MAP_TOKENS);
  return useMemo(() => paletteFrom(values), [values]);
}

/** `#rrggbb` for an RGB triple — for colours that are saved as hex data. */
export function rgbToHex([r, g, b]: RGB): string {
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

/** Series order offered when creating FCAs / routes (each saves its own hex), so the two read apart. */
const FCA_SERIES = [3, 5, 8, 7, 1, 4];
const ROUTE_SERIES = [1, 6, 2, 4, 5, 3];

const seriesHex = (p: MapPalette, order: number[]) => order.map((n) => rgbToHex(p.series[n - 1]));

/** FCA colour swatches (hex, from the series tokens) for the current theme. */
export function useFcaColors(): string[] {
  const p = useMapPalette();
  return useMemo(() => seriesHex(p, FCA_SERIES), [p]);
}

/** Route colour swatches (hex, from the series tokens) for the current theme. */
export function useRouteColors(): string[] {
  const p = useMapPalette();
  return useMemo(() => seriesHex(p, ROUTE_SERIES), [p]);
}

export const readFcaColors = () => seriesHex(readMapPalette(), FCA_SERIES);
export const readRouteColors = () => seriesHex(readMapPalette(), ROUTE_SERIES);

/** Aircraft glyph colour (`--map-aircraft`), read now. The theme argument is kept for older callers. */
export const aircraftColor = (_theme?: Theme): RGB => toRgb(readToken("map-aircraft"));

/** An RGB triple whose channels resolve a token each time they're read (legacy constant imports). */
function liveRgb(token: string): RGB {
  const out = [0, 0, 0] as RGB;
  for (const i of [0, 1, 2]) {
    Object.defineProperty(out, i, { get: () => toRgb(readToken(token))[i], enumerable: true });
  }
  return out;
}

/** Selected-flight highlight (`--map-highlight`). Prefer `useMapPalette().highlight` in components. */
export const HIGHLIGHT: RGB = liveRgb("map-highlight");

/** Filed-route colour (`--map-route`). Prefer `useMapPalette().route` in components. */
export const ROUTE: RGB = liveRgb("map-route");

/** Per-position ATC colours as CSS values (badges, pills, hover cards). */
export const ATC_COLORS: Record<string, string> = {
  DEL: "var(--atc-del)",
  GND: "var(--atc-gnd)",
  TWR: "var(--atc-twr)",
  APP: "var(--atc-app)",
  CTR: "var(--atc-ctr)",
  ATIS: "var(--atc-atis)",
};

/** Parse a saved `#rrggbb` colour to an [r, g, b] tuple (a bad value falls back to `--ink-3`). */
export function hexToRgb(hex: string): RGB {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return toRgb(readToken("ink-3"));
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

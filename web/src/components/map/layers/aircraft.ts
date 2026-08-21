import {IconLayer, TextLayer} from "@deck.gl/layers";

import {aircraftIconUrl} from "@/lib/aircraft-icons";
import type {Theme} from "../lib/constants";
import {aircraftColor, labelBackground, labelColor} from "../lib/colors";
import {TRIANGLE_ICON} from "../lib/icons";
import type {NormAircraft, RGB} from "../lib/types";

export interface AircraftLayerOptions {
  theme: Theme;
  /** Per-aircraft glyph color (defaults to the theme aircraft color). */
  getColor?: (a: NormAircraft) => RGB;
  /** Per-aircraft glyph size in pixels (default 26). */
  getSize?: (a: NormAircraft) => number;
  /** Glyph style: type silhouettes or plain triangles. */
  style?: "silhouette" | "triangle";
  /** Zoom-driven multiplier applied to the base pixel size (default 1 = constant size). */
  sizeScale?: number;
  /** Bumps updateTriggers when selection/highlight changes color/size. */
  highlightKey?: unknown;
}

/** Keep zoom-scaled glyphs legible without letting them balloon at extreme zoom. */
export function clampGlyphSize(px: number): number {
  return Math.max(9, Math.min(px, 56));
}

/** Heading-rotated aircraft glyphs (VATSIM-Radar type silhouettes, masked so they take `getColor`). */
export function buildAircraftLayer(data: NormAircraft[], opts: AircraftLayerOptions) {
  const base = aircraftColor(opts.theme);
  const triangle = opts.style === "triangle";
  const scale = opts.sizeScale ?? 1;
  return new IconLayer<NormAircraft>({
    id: "aircraft",
    data,
    pickable: true,
    getIcon: (d) => {
      if (triangle) return { id: "tri", url: TRIANGLE_ICON, width: 24, height: 24, mask: true };
      const url = aircraftIconUrl(d.actype);
      return { id: url, url, width: 48, height: 48, mask: true };
    },
    getPosition: (d) => [d.lon, d.lat],
    getAngle: (d) => 360 - d.heading,
    getColor: (d) => opts.getColor?.(d) ?? base,
    // Triangles fill their icon box (silhouettes have padding), so they read larger — render smaller.
    getSize: (d) => clampGlyphSize((opts.getSize?.(d) ?? (triangle ? 15 : 26)) * scale),
    sizeUnits: "pixels",
    billboard: false,
    updateTriggers: {
      getColor: [opts.theme, opts.highlightKey],
      getSize: [opts.highlightKey, triangle, scale],
      getIcon: [triangle],
    },
  });
}

export interface LabelFlags {
  callsign: boolean;
  type: boolean;
  alt: boolean;
  speed: boolean;
}

/** Aircraft text labels (callsign/type/alt/gs), stacked under each glyph. */
export function buildLabelLayer(data: NormAircraft[], labels: LabelFlags, theme: Theme) {
  return new TextLayer<NormAircraft>({
    id: "labels",
    data,
    getPosition: (d) => [d.lon, d.lat],
    getText: (d) => {
      const lines: string[] = [];
      if (labels.callsign) lines.push(d.callsign);
      if (labels.type && d.actype) lines.push(d.actype);
      if (labels.alt) lines.push(`${d.alt}ft`);
      if (labels.speed) lines.push(`${d.gs}kt`);
      return lines.join("\n");
    },
    getColor: labelColor(theme),
    getSize: 11,
    getPixelOffset: [0, 16],
    getTextAnchor: "middle",
    getAlignmentBaseline: "top",
    background: true,
    getBackgroundColor: labelBackground(theme),
    backgroundPadding: [3, 1],
    updateTriggers: {
      getText: [labels.callsign, labels.type, labels.alt, labels.speed],
      getColor: [theme],
      getBackgroundColor: [theme],
    },
  });
}

import type {Theme} from "./constants";
import type {RGB, RGBA} from "./types";

/** Aircraft glyph color per theme (amber on dark, slate on light). */
export const aircraftColor = (theme: Theme): RGB => (theme === "dark" ? [255, 190, 70] : [40, 60, 90]);

/** Selected-flight highlight (also used for its flown trail). */
export const HIGHLIGHT: RGB = [56, 189, 248];

/** Filed-route violet (distinct from the flown trail). */
export const ROUTE: RGB = [167, 139, 250];

export const boundaryColor = (theme: Theme): RGBA =>
  theme === "dark" ? [130, 140, 160, 110] : [90, 100, 120, 120];

export const labelColor = (theme: Theme): RGB => (theme === "dark" ? [230, 235, 245] : [20, 25, 35]);

export const labelBackground = (theme: Theme): RGBA =>
  theme === "dark" ? [10, 12, 16, 180] : [255, 255, 255, 190];

export const waypointBackground = (theme: Theme): RGBA =>
  theme === "dark" ? [10, 12, 16, 200] : [255, 255, 255, 210];

/** FCA polyline palette (cycled when creating; each FCA stores its own color). */
export const FCA_COLORS = ["#f59e0b", "#ec4899", "#84cc16", "#f97316", "#38bdf8", "#f87171"];

/** Route palette so named routes read differently from FCAs. */
export const ROUTE_COLORS = ["#38bdf8", "#22d3ee", "#34d399", "#a78bfa", "#f472b6", "#facc15"];

/** Per-facility ATC colors (badges, TRACON/center areas). */
export const ATC_COLORS: Record<string, string> = {
  DEL: "#60a5fa",
  GND: "#4ade80",
  TWR: "#f87171",
  APP: "#fb923c",
  CTR: "#2dd4bf",
  ATIS: "#facc15",
};

/** Parse a `#rrggbb` string to an [r, g, b] tuple (falls back to slate on a bad value). */
export function hexToRgb(hex: string): RGB {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return [148, 163, 184];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

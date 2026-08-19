/** Shared map constants for the deck.gl + MapLibre map stack (replay, FCA, dashboard, advisories). */

/** Free CARTO vector basemap styles (no access token needed). */
export const CARTO_STYLE = {
  dark: "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json",
  light: "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json",
} as const;

/** Default camera: centered on the CONUS. */
export const US_HOME = { longitude: -98.35, latitude: 39.5, zoom: 3.4 };

/** Per-theme colors for the airport-layout (aeroway) overlay: apron fill, taxiway, runway. */
export const AEROWAY_COLORS = {
  dark: { fill: "#20242e", taxiway: "#4a5162", runway: "#8a93a6" },
  light: { fill: "#e3e7ee", taxiway: "#c4cad4", runway: "#98a1b2" },
} as const;

export type Theme = "dark" | "light";

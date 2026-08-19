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

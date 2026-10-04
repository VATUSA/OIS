/**
 * Rules for a persisted, user-chosen colour (an FCA, a route, a facility-map rule) — DESIGN.md
 * § "User-chosen domain colour". The backend enforces the same rule (`repos::flow::fca_color`).
 */

/** The dark theme's `--ground`: the background a user colour must stay visible on. */
export const DARK_GROUND = "#08080a";

/** The minimum WCAG contrast a user colour must have against [`DARK_GROUND`]. */
export const MIN_GROUND_CONTRAST = 3;

/** `value` as stored — trimmed, lowercase `#rrggbb` — or `null` for any other shape. */
export function normalizeHex(value: string): string | null {
  const v = value.trim().toLowerCase();
  return /^#[0-9a-f]{6}$/.test(v) ? v : null;
}

function luminance(hex: string): number {
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const [r, g, b] = [1, 3, 5].map((i) => lin(parseInt(hex.slice(i, i + 2), 16) / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio between two `#rrggbb` colours. */
export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Whether a colour is valid to store: `#rrggbb` and visible on the dark ground. */
export function meetsGroundFloor(value: string): boolean {
  const hex = normalizeHex(value);
  return hex !== null && contrastRatio(hex, DARK_GROUND) >= MIN_GROUND_CONTRAST;
}

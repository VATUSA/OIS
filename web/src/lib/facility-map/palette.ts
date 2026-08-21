/**
 * The shared color palette for facility-map aircraft coloring. A fixed, named set (à la VATUSA's
 * legacy TMU map buckets) so rule colors are consistent, legible on both basemaps, and shareable as
 * plain hex. Rules store the hex; the map/legend look up the label.
 */

export interface PaletteColor {
  hex: string;
  label: string;
}

export const PALETTE: PaletteColor[] = [
  { hex: "#e5484d", label: "Red" },
  { hex: "#f76b15", label: "Orange" },
  { hex: "#e3b341", label: "Amber" },
  { hex: "#57ab5a", label: "Green" },
  { hex: "#39c5cf", label: "Cyan" },
  { hex: "#4c8dff", label: "Blue" },
  { hex: "#8b5cf6", label: "Purple" },
  { hex: "#e668c6", label: "Pink" },
  { hex: "#a0785a", label: "Brown" },
  { hex: "#8b949e", label: "Gray" },
];

const LABEL_BY_HEX = new Map(PALETTE.map((c) => [c.hex.toLowerCase(), c.label]));

/** Human label for a palette hex (falls back to the hex itself for custom/legacy colors). */
export function colorLabel(hex: string): string {
  return LABEL_BY_HEX.get(hex.toLowerCase()) ?? hex;
}

/** The default swatch offered for a new rule. */
export const DEFAULT_RULE_COLOR = PALETTE[0].hex;

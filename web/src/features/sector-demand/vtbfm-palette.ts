import type {CSSProperties} from "react";

/**
 * vTBFM's Sector Monitor look, copied value for value (#794). This file is the one place in OIS where
 * the Sector Monitor's body takes literal colours, a font stack and bevels instead of tokens: a named
 * exception in DESIGN.md § "Named exception: the Sector Monitor body", which the owner chose so the
 * page matches the monitor controllers already use. `colours.guard.test.ts` exempts this file and no
 * other, so every literal the page draws comes from here.
 *
 * Source: zla-artcc/vTBFM `b7328138`, `src/components/SectorMonitorPage.tsx:22-38, 69-74` (the page)
 * and `src/components/SectorContextMenu.tsx:77-127, 202-226` (the menu).
 */

export const C = {
  beige: "#EFDFCE",
  face: "#CAC7C9",
  text: "#000000",
  sectorCyan: "#94C9D8",
  footerCyan: "#86CEE6",
  sliderCyan: "#70D6E7",
  palePink: "#D9CBC4",
  green: "#3DC744",
  yellow: "#F2F72A",
  red: "#D90F10",
  gridLine: "#D8ECE0",
  outer: "#526663",
  field: "#ffffff",
  error: "#D90F10",
} as const;

/** The monitor's monospace ("ancient") stack. */
export const FONT = "ui-monospace, 'Cascadia Mono', 'Segoe UI Mono', 'SF Mono', Consolas, monospace";
/** vTBFM's weight for every label and figure on the page. */
export const WEIGHT = 500;
export const FS = 14;
export const FS_SM = 11;
export const FS_LG = 15;
/** The sector name and MAP columns, a body row, and the footer row. */
export const NAME_W = 54;
export const MAP_W = 44;
export const ROW_H = 30;
export const FOOT_H = 28;
export const MAP_INPUT_W = 34;

export const raised = (p = 2): CSSProperties => ({border: `${p}px outset ${C.face}`, background: C.face});
export const inset = (p = 1): CSSProperties => ({border: `${p}px inset ${C.face}`});
export const gridBorder = `1px solid ${C.gridLine}`;
export const footerBorder = `2px solid ${C.text}`;

/** The slider's two-tone track: cyan up to the thumb, pale pink after, hard stops. */
export const sliderTrack = (pct: number) =>
  `linear-gradient(to right, ${C.sliderCyan} 0%, ${C.sliderCyan} ${pct}%, ${C.palePink} ${pct}%, ${C.palePink} 100%)`;

/** The slider thumb and focus ring, which inline styles can't reach. */
export const RETRO_CSS = `
.vtbfm-range{ -webkit-appearance:none; appearance:none; height:12px; border:1px inset ${C.face}; }
.vtbfm-range::-webkit-slider-thumb{ -webkit-appearance:none; appearance:none; width:7px; height:14px; margin-top:-1px; background:#6b6b6b; border:1px solid #202020; cursor:pointer; }
.vtbfm-range::-moz-range-thumb{ width:7px; height:14px; background:#6b6b6b; border:1px solid #202020; border-radius:0; cursor:pointer; }
.vtbfm-focus:focus-visible{ outline:1px dotted #111; outline-offset:1px; }
`;

// --- The context menu (SectorContextMenu.tsx:77-127) --------------------------------------------

export const MENU = {
  fontPx: 14,
  rowH: 30,
  textInset: 7,
  arrowRight: 10,
  arrowCol: 18,
  checkCol: 16,
  bevelTL: 3,
  bevelBR: 4,
  edge: 2,
  submenuOpenMs: 180,
  surface: "#fafad2",
  text: "#000000",
  disabledText: "#8a8a72",
  rowRaisedBg: "#fefee2",
  arrowFace: "#dcdcb4",
  arrowLight: "#fdfdea",
  arrowDark: "#6e6e5a",
} as const;

const UP = ["#ffffff", "#fefef0", "#fcfce3"];
const DOWN = ["#33332a", "#6e6e5a", "#a3a385", "#cfcfae"];

/** The menu's raised control edge: stacked 1px inset bands, outermost first, no blur. */
export const MENU_BEVEL = [
  ...UP.map((c, i) => `inset 0 ${i + 1}px 0 0 ${c}`),
  ...DOWN.map((c, i) => `inset 0 -${i + 1}px 0 0 ${c}`),
  ...UP.map((c, i) => `inset ${i + 1}px 0 0 0 ${c}`),
  ...DOWN.map((c, i) => `inset -${i + 1}px 0 0 0 ${c}`),
].join(", ");

/** A hovered menu row: the same construction at half the depth. */
export const MENU_ROW_RAISED = [
  "inset 0 1px 0 0 #ffffff",
  "inset 0 2px 0 0 #fdfde9",
  "inset 0 -1px 0 0 #8c8c72",
  "inset 0 -2px 0 0 #bcbc9c",
  "inset 1px 0 0 0 #ffffff",
  "inset 2px 0 0 0 #fdfde9",
  "inset -1px 0 0 0 #8c8c72",
  "inset -2px 0 0 0 #bcbc9c",
].join(", ");

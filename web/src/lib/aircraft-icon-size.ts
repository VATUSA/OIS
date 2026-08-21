// Per-type relative glyph size, ported from VATSIM Radar's `aircraftIcons` coefficients
// (vatsim-radar app/utils/icons.ts). Without this every silhouette renders at the same box, so a
// Cessna 172 looks as big as an A380. VATSIM Radar sizes each type as `round(30 * coef)` clamped to
// [12, 35] px; we reproduce that and normalize against a mid-size airliner (the A320) so common jets
// keep their current size while GA shrinks and heavies grow.

import {resolveIconKey} from "./aircraft-icons";

/** icon key → VATSIM Radar size coefficient. Keys match our silhouette files. */
const ICON_COEF: Record<string, number> = {
  a10: 0.29, a124: 1.22, a139: 0.23, a149: 0.24, a169: 0.2, a189: 0.24, a20n: 0.57, a225: 1.46,
  a300: 0.75, a310: 0.73, a318: 0.6, a319: 0.6, a320: 0.6, a321: 0.6, a332: 1.01, a333: 1.01,
  a338: 1.07, a339: 1.07, a342: 1.01, a343: 1.01, a345: 1.06, a346: 1.06, a359: 1.08, a35k: 1.08,
  a388: 1.33, a3st: 0.75, a400: 0.71, a748: 0.5, aa5: 0.16, aest: 0.19, an2: 0.3, an24: 0.49,
  an26: 0.49, as32: 0.27, as50: 0.14, at4x: 0.38, at7x: 0.45, atp: 0.51, b06: 0.08, b1: 0.48,
  b105: 0.16, b190: 0.29, b2: 0.87, b350: 0.29, b36t: 0.17, b37m: 0.6, b38m: 0.6, b39m: 0.6,
  b407: 0.18, b461: 0.44, b462: 0.44, b463: 0.44, b52: 0.94, b703: 0.74, b712: 0.47, b720: 0.66,
  b721: 0.55, b722: 0.55, b731: 0.47, b732: 0.47, b733: 0.48, b734: 0.48, b735: 0.48, b736: 0.57,
  b737: 0.57, b738: 0.57, b739: 0.57, b741: 0.99, b744: 1.07, b748: 1.14, b74s: 0.99, b752: 0.63,
  b753: 0.63, b762: 0.79, b763: 0.79, b764: 0.87, b772: 1.02, b773: 1.02, b77l: 1.08, b77w: 1.08,
  b788: 1, b789: 1, b78x: 1, ba11: 0.45, ball: 0.57, bcs1: 0.59, bcs3: 0.59, be20: 0.29,
  be33: 0.17, be35: 0.17, be36: 0.17, be58: 0.19, be60: 0.2, be9l: 0.26, blcf: 1.07, bn2p: 0.25,
  br23: 0.14, bt7: 0.16, c130: 0.67, c152: 0.17, c160: 0.67, c17: 0.86, c172: 0.18, c2: 0.41,
  c206: 0.18, c208: 0.26, c25b: 0.27, c25c: 0.26, c310: 0.18, c402: 0.22, c414: 0.22, c46: 0.55,
  c510: 0.22, c525: 0.24, c5m: 1.13, c68a: 0.37, c700: 0.35, c750: 0.32, c919: 0.6, cl2t: 0.47,
  cl30: 0.35, cl60: 0.33, cp10: 0.13, crj2: 0.35, crj7: 0.39, crj9: 0.42, crjx: 0.44, cs50: 0.27,
  da40: 0.19, da42: 0.23, da62: 0.24, dc10: 0.84, dc3: 0.48, dc6: 0.6, dc86: 0.72, dh88: 0.22,
  dh8a: 0.43, dh8c: 0.46, dh8d: 0.47, dhc2: 0.24, dhc6: 0.33, dhc7: 0.47, dimo: 0.13, dr40: 0.15,
  dv20: 0.18, e135: 0.33, e145: 0.33, e170: 0.43, e175: 0.43, e190: 0.48, e195: 0.48, e2: 0.41,
  e290: 0.56, e295: 0.59, e300: 0.13, e3cf: 0.74, e50p: 0.21, e55p: 0.27, e75s: 0.43, ec45: 0.18,
  eh10: 0.29, epic: 0.22, eufi: 0.18, evot: 0.18, f100: 0.47, f104: 0.11, f117: 0.22, f14: 0.19,
  f15: 0.22, f16: 0.17, f18: 0.21, f22: 0.23, f27: 0.48, f28: 0.42, f2th: 0.36, f35: 0.18,
  f70: 0.47, f900: 0.32, fa10: 0.22, fa20: 0.27, fa50: 0.31, fa6x: 0.43, fa7x: 0.44, fa8x: 0.44,
  fdct: 0.14, fox: 0.16, g109: 0.8, g2ca: 0.11, g91: 0.14, ga7c: 0.52, gl5t: 0.48, gl7t: 0.53,
  glex: 0.48, glf5: 0.47, glf6: 0.51, glid: 0.25, h160: 0.22, h47: 0.27, h60: 0.21, h64: 0.18,
  hawk: 0.17, hdjt: 0.2, hunt: 0.17, husk: 0.18, il18: 0.62, il20: 0.62, il22: 0.38, il76: 0.84,
  il96: 1, j328: 0.35, js41: 0.3, k35e: 0.66, kodi: 0.23, l101: 0.79, l159: 0.16, l39: 0.16,
  l410: 0.33, lanc: 0.52, leg2: 0.13, lj35: 0.2, m20p: 0.19, md11: 0.85, md80: 0.53, me08: 0.18,
  me09: 0.17, me62: 0.21, mir2: 0.15, p06t: 0.19, p180: 0.23, p212: 0.23, p28x: 0.15, p46t: 0.22,
  p51: 0.19, pa18: 0.18, pa24: 0.18, pa34: 0.2, pa38: 0.17, pa39: 0.19, pa44: 0.2, pc12: 0.27,
  pc21: 0.23, pc24: 0.3, pc6t: 0.26, pite: 0.8, pts1: 0.09, r66: 0.07, rfal: 0.18, rv10: 0.16,
  s12s: 0.8, s76: 0.22, s92: 0.21, savg: 0.16, sb20: 0.41, sb39: 0.14, sf34: 0.36, sf50: 0.2,
  sh36: 0.38, ship: 0.67, shrk: 0.13, sira: 0.14, spit: 0.19, sr22: 0.05, sr71: 0.28, star: 0.28,
  su27: 0.25, su95: 0.46, sw3: 0.29, t134: 0.48, t154: 0.63, tbm7: 0.21, tbm8: 0.21, tbm9: 0.21,
  tex2: 0.18, tl20: 0.15, tor: 0.11, tris: 0.25, twen: 0.17, u2: 0.52, uh1: 0.17, v10: 0.2,
  v22: 0.43, vamp: 0.19, vc10: 0.76, vipj: 0.14, visc: 0.48, vl3: 0.14, vulc: 0.51, yk40: 0.42,
  // OIS-only silhouettes with no VATSIM Radar coefficient — hand-picked to sit sensibly in the range.
  conc: 0.72, f4: 0.3, r22: 0.06, r44: 0.08, t144: 0.8,
};

const STANDARD_COEF = 30;
const MIN_WIDTH = 12;
const MAX_WIDTH = 35;

/** VATSIM Radar's clamped per-type pixel width for a coefficient. */
function widthForCoef(coef: number): number {
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.round(STANDARD_COEF * coef)));
}

/** The A320 is the neutral 1.0 reference so common airliners keep their tuned base size. */
const REF_WIDTH = widthForCoef(ICON_COEF.a320);

/**
 * Relative size multiplier for a type's silhouette (≈0.67 for light GA, 1.0 for a narrowbody, ~1.9 for
 * the largest heavies). Unknown types fall back to 1. Applies only to silhouettes — triangle glyphs are
 * abstract markers and stay uniform.
 */
export function aircraftTypeScale(actype: string | null | undefined): number {
  const coef = ICON_COEF[resolveIconKey(actype)];
  if (coef === undefined) return 1;
  return widthForCoef(coef) / REF_WIDTH;
}

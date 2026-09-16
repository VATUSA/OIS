import {useMemo} from "react";
import {parseColor, readToken, useTokens} from "@ois/ui";

/**
 * The swatches offered for facility-map aircraft coloring: a fixed, named set drawn from the
 * `--series-*` tokens (plus a neutral grey), resolved to hex. Rules store the hex; the map/legend look
 * up the label, which also recognises the other theme's values and the pre-token palette.
 */

export interface PaletteColor {
  hex: string;
  label: string;
}

const SWATCHES = [
  { token: "series-5", label: "Red" },
  { token: "series-7", label: "Orange" },
  { token: "series-3", label: "Amber" },
  { token: "series-8", label: "Lime" },
  { token: "series-2", label: "Green" },
  { token: "series-1", label: "Cyan" },
  { token: "series-6", label: "Blue" },
  { token: "series-4", label: "Purple" },
  { token: "ink-3", label: "Gray" },
] as const;

const TOKENS = SWATCHES.map((s) => s.token);

/** Hex rules were saved with before the swatches came from tokens — kept so saved colours keep their names. */
const LEGACY_LABELS: Record<string, string> = {
  "#e5484d": "Red",
  "#f76b15": "Orange",
  "#e3b341": "Amber",
  "#57ab5a": "Green",
  "#39c5cf": "Cyan",
  "#4c8dff": "Blue",
  "#8b5cf6": "Purple",
  "#e668c6": "Pink",
  "#a0785a": "Brown",
  "#8b949e": "Gray",
};

function toHex(value: string): string {
  const [r, g, b] = parseColor(value);
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

const build = (values: Record<string, string>): PaletteColor[] =>
  SWATCHES.map((s) => ({ hex: toHex(values[s.token]), label: s.label }));

/** The rule swatches for the current theme, re-read on a theme switch. */
export function useRulePalette(): PaletteColor[] {
  const values = useTokens(TOKENS);
  return useMemo(() => build(values), [values]);
}

/** The rule swatches for the current theme, read now. */
export function readRulePalette(): PaletteColor[] {
  return build(Object.fromEntries(TOKENS.map((t) => [t, readToken(t)])));
}

/** The default swatch offered for a new rule. */
export const defaultRuleColor = (palette: PaletteColor[] = readRulePalette()) => palette[0].hex;

/** Every theme's declared value of each swatch token (from the loaded stylesheets), hex → label. */
function tokenLabelsAllThemes(): Map<string, string> {
  const out = new Map<string, string>();
  const visit = (rules: CSSRuleList) => {
    for (const rule of Array.from(rules)) {
      if (rule instanceof CSSStyleRule) {
        for (const s of SWATCHES) {
          const v = rule.style.getPropertyValue(`--${s.token}`).trim();
          if (v) out.set(toHex(v), s.label);
        }
      } else if ("cssRules" in rule) {
        visit((rule as CSSGroupingRule).cssRules);
      }
    }
  };
  if (typeof document !== "undefined") {
    for (const sheet of Array.from(document.styleSheets)) {
      try {
        visit(sheet.cssRules);
      } catch {
        // A cross-origin sheet can't be read; the app's own tokens are same-origin.
      }
    }
  }
  return out;
}

let labels: Map<string, string> | null = null;

/** Human label for a rule hex (falls back to the hex itself for custom colors). */
export function colorLabel(hex: string): string {
  const key = hex.toLowerCase();
  if (!labels || labels.size === 0) labels = tokenLabelsAllThemes();
  return labels.get(key) ?? LEGACY_LABELS[key] ?? readRulePalette().find((c) => c.hex === key)?.label ?? hex;
}

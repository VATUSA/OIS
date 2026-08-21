/**
 * Client-side aircraft color-rule engine for the facility map. Turns a facility's stored config into a
 * `(aircraft) => RGB` function for the map's `getAircraftColor`. Rules are evaluated in order; the first
 * enabled rule whose conditions ALL match wins. Unmatched aircraft get the config's default color, or
 * the supplied theme fallback when no default is set.
 */

import type {components} from "@ois/api-client";

import {hexToRgb} from "@/components/map/lib/colors";
import type {NormAircraft, RGB} from "@/components/map/lib/types";

export type FacilityMapConfig = components["schemas"]["FacilityMapConfigBody"];
export type ColorRule = components["schemas"]["ColorRule"];
export type RuleCondition = components["schemas"]["RuleCondition"];

/** The flight attributes a condition can match on, with UI labels. */
export const RULE_FIELDS = [
  { value: "arr", label: "Arrival airport" },
  { value: "star", label: "STAR / arrival gate" },
  { value: "dep", label: "Departure airport" },
  { value: "type", label: "Aircraft type" },
  { value: "wake", label: "Wake category" },
  { value: "rules", label: "Flight rules" },
  { value: "alt", label: "Filed altitude (ft)" },
] as const;

/** The numeric fields (use range/lt/gt); everything else is a string match (in/prefix). */
const NUMERIC_FIELDS = new Set(["alt"]);

export function isNumericField(field: string): boolean {
  return NUMERIC_FIELDS.has(field);
}

function fieldValue(a: NormAircraft, field: string): string | number | undefined {
  switch (field) {
    case "arr":
      return a.arr;
    case "dep":
      return a.dep;
    case "star":
      return a.star ?? "";
    case "type":
      return a.actype;
    case "wake":
      return a.wake ?? "";
    case "rules":
      return a.flightRules ?? "";
    case "alt":
      return a.filedAlt ?? 0;
    default:
      return undefined;
  }
}

function matchCondition(a: NormAircraft, c: RuleCondition): boolean {
  const v = fieldValue(a, c.field);
  if (v === undefined) return false;
  const vals = c.values;
  switch (c.op) {
    case "eq":
    case "in": {
      const s = String(v).toUpperCase();
      return vals.some((x) => x.toUpperCase() === s);
    }
    case "prefix": {
      const s = String(v).toUpperCase();
      return vals.some((x) => x !== "" && s.startsWith(x.toUpperCase()));
    }
    case "lt":
      return Number(v) < Number(vals[0]);
    case "gt":
      return Number(v) > Number(vals[0]);
    case "range":
      return Number(v) >= Number(vals[0]) && Number(v) <= Number(vals[1]);
    default:
      return false;
  }
}

/**
 * Build the map color function for a facility config. `fallback` (the theme aircraft color) is used
 * when no rule matches and the config sets no default color.
 */
export function buildColorFn(
  config: { rules?: ColorRule[]; default_color?: string } | undefined,
  fallback: RGB,
): (a: NormAircraft) => RGB {
  // Only enabled rules with at least one condition (a rule with no conditions would match everything,
  // which is what the default color is for).
  const rules = (config?.rules ?? []).filter((r) => r.enabled && r.conditions.length > 0);
  const cache = new Map<string, RGB>();
  const rgb = (hex: string): RGB => {
    let c = cache.get(hex);
    if (!c) {
      c = hexToRgb(hex);
      cache.set(hex, c);
    }
    return c;
  };
  const def = config?.default_color ? rgb(config.default_color) : fallback;
  return (a) => {
    for (const r of rules) {
      if (r.conditions.every((c) => matchCondition(a, c))) return rgb(r.color);
    }
    return def;
  };
}

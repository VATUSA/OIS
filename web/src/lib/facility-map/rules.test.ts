import {describe, expect, it} from "vitest";

import {buildColorFn, type FacilityMapConfig, type ColorRule} from "./rules";
import type {NormAircraft, RGB} from "@/components/map/lib/types";

const FALLBACK: RGB = [1, 2, 3];

function ac(over: Partial<NormAircraft>): NormAircraft {
  return {
    id: "x",
    callsign: "AAL1",
    actype: "B738",
    dep: "KDCA",
    arr: "KIAD",
    lat: 0,
    lon: 0,
    alt: 0,
    gs: 0,
    heading: 0,
    ...over,
  };
}

function rule(over: Partial<ColorRule>): ColorRule {
  return { id: "r", label: "", color: "#ff0000", enabled: true, conditions: [], ...over };
}

function cfg(rules: ColorRule[], default_color = ""): FacilityMapConfig {
  return { facility_id: "ZDC", rules, default_color, editable: false };
}

const RED: RGB = [255, 0, 0];
const BLUE: RGB = [0, 0, 255];

describe("buildColorFn", () => {
  it("matches arrival airport (eq/in) and returns the rule color", () => {
    const fn = buildColorFn(
      cfg([rule({ color: "#ff0000", conditions: [{ field: "arr", op: "in", values: ["KIAD"] }] })]),
      FALLBACK,
    );
    expect(fn(ac({ arr: "KIAD" }))).toEqual(RED);
    expect(fn(ac({ arr: "KBWI" }))).toEqual(FALLBACK);
  });

  it("matches STAR case-insensitively", () => {
    const fn = buildColorFn(
      cfg([rule({ conditions: [{ field: "star", op: "in", values: ["cavlr"] }] })]),
      FALLBACK,
    );
    expect(fn(ac({ star: "CAVLR" }))).toEqual(RED);
    expect(fn(ac({ star: null }))).toEqual(FALLBACK);
  });

  it("supports prefix matching (airport/type)", () => {
    const fn = buildColorFn(
      cfg([rule({ conditions: [{ field: "dep", op: "prefix", values: ["K"] }] })]),
      FALLBACK,
    );
    expect(fn(ac({ dep: "KDCA" }))).toEqual(RED);
    expect(fn(ac({ dep: "EGLL" }))).toEqual(FALLBACK);
  });

  it("supports numeric altitude range/lt/gt on filed altitude", () => {
    const range = buildColorFn(
      cfg([rule({ conditions: [{ field: "alt", op: "range", values: ["10000", "24000"] }] })]),
      FALLBACK,
    );
    expect(range(ac({ filedAlt: 18000 }))).toEqual(RED);
    expect(range(ac({ filedAlt: 35000 }))).toEqual(FALLBACK);

    const below = buildColorFn(
      cfg([rule({ conditions: [{ field: "alt", op: "lt", values: ["10000"] }] })]),
      FALLBACK,
    );
    expect(below(ac({ filedAlt: 5000 }))).toEqual(RED);
    expect(below(ac({ filedAlt: 15000 }))).toEqual(FALLBACK);
  });

  it("ANDs conditions within a rule", () => {
    const fn = buildColorFn(
      cfg([
        rule({
          conditions: [
            { field: "arr", op: "in", values: ["KIAD"] },
            { field: "wake", op: "in", values: ["H"] },
          ],
        }),
      ]),
      FALLBACK,
    );
    expect(fn(ac({ arr: "KIAD", wake: "H" }))).toEqual(RED);
    expect(fn(ac({ arr: "KIAD", wake: "M" }))).toEqual(FALLBACK);
  });

  it("first enabled matching rule wins; disabled rules are skipped", () => {
    const fn = buildColorFn(
      cfg([
        rule({ id: "a", color: "#0000ff", enabled: false, conditions: [{ field: "arr", op: "in", values: ["KIAD"] }] }),
        rule({ id: "b", color: "#ff0000", conditions: [{ field: "arr", op: "in", values: ["KIAD"] }] }),
      ]),
      FALLBACK,
    );
    expect(fn(ac({ arr: "KIAD" }))).toEqual(RED);
  });

  it("uses the default color for unmatched aircraft when set, else the fallback", () => {
    const withDefault = buildColorFn(
      cfg([rule({ conditions: [{ field: "arr", op: "in", values: ["KIAD"] }] })], "#0000ff"),
      FALLBACK,
    );
    expect(withDefault(ac({ arr: "KBWI" }))).toEqual(BLUE);
    const noDefault = buildColorFn(cfg([]), FALLBACK);
    expect(noDefault(ac({}))).toEqual(FALLBACK);
  });
});

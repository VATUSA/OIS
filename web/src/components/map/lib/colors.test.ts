import {describe, expect, it} from "vitest";

import {hexToRgb, rgbToHex, swatchCss} from "./colors";

describe("hexToRgb", () => {
  it("parses #rrggbb in either case, with or without the #", () => {
    expect(hexToRgb("#efc14d")).toEqual([239, 193, 77]);
    expect(hexToRgb("#EFC14D")).toEqual([239, 193, 77]);
    expect(hexToRgb(" efc14d ")).toEqual([239, 193, 77]);
  });

  it("falls back to the neutral for any other shape", () => {
    const fallback = hexToRgb("not a colour");
    for (const bad of ["#abc", "red", "#efc14d00", "rgb(1, 2, 3)", ""]) {
      expect(hexToRgb(bad), bad).toEqual(fallback);
    }
  });
});

describe("rgbToHex", () => {
  it("round-trips through hexToRgb as lowercase #rrggbb", () => {
    for (const hex of ["#efc14d", "#000000", "#ffffff", "#5ec8e5"]) {
      expect(rgbToHex(hexToRgb(hex))).toBe(hex);
    }
  });
});

/**
 * The list chip and the map agree on every value (#698). An accepted colour — what the server stores,
 * lowercase `#rrggbb` — shows as itself in both; anything else shows the map's grey in the list too,
 * where a raw `style` showed `#abc` or `red` correctly while the map drew grey.
 */
describe("swatchCss", () => {
  it("shows an accepted colour as itself, as the map draws it", () => {
    for (const hex of ["#efc14d", "#5ec8e5", "#6b6b74", "#3565d6"]) {
      expect(swatchCss(hex)).toBe(hex);
      expect(swatchCss(hex)).toBe(rgbToHex(hexToRgb(hex)));
    }
  });

  it("shows a value the map can't parse as the map's fallback, not as itself", () => {
    for (const bad of ["#abc", "red", "#efc14d00"]) {
      expect(swatchCss(bad)).toBe(rgbToHex(hexToRgb(bad)));
      expect(swatchCss(bad)).not.toBe(bad);
    }
  });
});

import {describe, expect, it} from "vitest";

import {contrastRatio, DARK_GROUND, meetsGroundFloor, normalizeHex} from "./colour";

describe("normalizeHex", () => {
  it("trims and lowercases #rrggbb, and refuses anything else", () => {
    expect(normalizeHex(" #EFC14D ")).toBe("#efc14d");
    for (const bad of ["red", "#abc", "efc14d", "#efc14d00", "#gggggg"]) {
      expect(normalizeHex(bad), bad).toBeNull();
    }
  });
});

describe("the ground-contrast floor (#698)", () => {
  it("measures WCAG contrast", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrastRatio(DARK_GROUND, DARK_GROUND)).toBeCloseTo(1, 5);
  });

  it("refuses a colour that would vanish on the dark ground", () => {
    for (const dark of ["#000000", "#08080a", "#333333", "#454545"]) {
      expect(meetsGroundFloor(dark), dark).toBe(false);
    }
  });

  it("accepts every token swatch, in both themes", () => {
    const tokens = [
      "#1b8fb0", "#1f9d63", "#b7791f", "#8e5bd0", "#d0556b", "#3565d6", "#c2621a", "#5f8f2a", "#9898a2",
      "#5ec8e5", "#43d089", "#efc14d", "#c792ea", "#f07178", "#7b9dff", "#f5a83d", "#a3d977", "#6b6b74",
    ];
    for (const hex of tokens) expect(meetsGroundFloor(hex), hex).toBe(true);
  });
});

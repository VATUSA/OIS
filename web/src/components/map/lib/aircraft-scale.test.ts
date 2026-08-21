import {describe, expect, it} from "vitest";

import {zoomAircraftScale} from "./aircraft-scale";

describe("zoomAircraftScale", () => {
  it("is 1 at the reference zoom (glyphs render at base size)", () => {
    expect(zoomAircraftScale(6)).toBeCloseTo(1, 5);
  });

  it("shrinks glyphs when zoomed out and grows them when zoomed in", () => {
    expect(zoomAircraftScale(3.4)).toBeLessThan(1);
    expect(zoomAircraftScale(2)).toBeLessThan(zoomAircraftScale(3.4));
    expect(zoomAircraftScale(10)).toBeGreaterThan(1);
    expect(zoomAircraftScale(14)).toBeGreaterThan(zoomAircraftScale(10));
  });

  it("increases monotonically with zoom", () => {
    let prev = -Infinity;
    for (let z = 2; z <= 20; z += 1) {
      const s = zoomAircraftScale(z);
      expect(s).toBeGreaterThanOrEqual(prev);
      prev = s;
    }
  });

  it("clamps out-of-range zooms and handles a non-finite zoom", () => {
    expect(zoomAircraftScale(-5)).toBe(zoomAircraftScale(2));
    expect(zoomAircraftScale(99)).toBe(zoomAircraftScale(20));
    expect(zoomAircraftScale(NaN)).toBe(1);
  });
});

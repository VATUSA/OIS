import {describe, expect, it} from "vitest";

import {aircraftTypeScale} from "./aircraft-icon-size";

describe("aircraftTypeScale", () => {
  it("makes a heavy larger than an airliner, and an airliner larger than light GA", () => {
    const c172 = aircraftTypeScale("C172");
    const a320 = aircraftTypeScale("A320");
    const a388 = aircraftTypeScale("A388");
    expect(c172).toBeLessThan(a320);
    expect(a320).toBeLessThan(a388);
    // The A320 is the neutral 1.0 reference.
    expect(a320).toBeCloseTo(1, 5);
  });

  it("strips the equipment suffix and is case-insensitive", () => {
    expect(aircraftTypeScale("b744/h")).toBe(aircraftTypeScale("B744"));
  });

  it("resolves aliased types (C182 shares the C172 silhouette)", () => {
    expect(aircraftTypeScale("C182")).toBe(aircraftTypeScale("C172"));
  });

  it("falls back to 1 for unknown or empty types", () => {
    expect(aircraftTypeScale("ZZZZ")).toBe(1); // resolves to the a320 fallback → 1
    expect(aircraftTypeScale(null)).toBe(1);
    expect(aircraftTypeScale("")).toBe(1);
  });
});

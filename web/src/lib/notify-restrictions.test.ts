import {describe, expect, it} from "vitest";

import {routeFor} from "./notify-restrictions";

describe("routeFor", () => {
  // Each alert kind opens the tab that lists it; a wrong mapping lands the click on the wrong list.
  it("sends each restriction kind to its own TMU tab", () => {
    expect(routeFor("gs:12")).toBe("/ops/tmu?tab=ground-stops");
    expect(routeFor("gdp:7")).toBe("/ops/tmu?tab=gdp");
    expect(routeFor("tmi:3")).toBe("/ops/tmu?tab=restrictions");
    expect(routeFor("prog:KATL")).toBe("/ops/tmu?tab=programs");
  });

  it("falls back to the restrictions tab for an unknown kind", () => {
    expect(routeFor("mystery:1")).toBe("/ops/tmu?tab=restrictions");
  });
});

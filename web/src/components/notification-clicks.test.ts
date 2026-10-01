import {describe, expect, it} from "vitest";

import {toNavigation} from "./notification-clicks";

describe("toNavigation", () => {
  // A query string left inside `to` matches no route, so the click landed on not-found (#348 review).
  it("hands a route's query string over as search, not as part of the path", () => {
    expect(toNavigation("/ops/tmu?tab=ground-stops")).toEqual({
      to: "/ops/tmu",
      search: {tab: "ground-stops"},
    });
    expect(toNavigation("/ops/fca?fca=ZDC%201")).toEqual({to: "/ops/fca", search: {fca: "ZDC 1"}});
  });

  it("leaves a plain path alone", () => {
    expect(toNavigation("/planning/events/9")).toEqual({to: "/planning/events/9"});
  });
});

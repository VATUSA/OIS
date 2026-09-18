import {describe, expect, it} from "vitest";

import {pageTitle} from "./page-meta";

describe("pageTitle", () => {
  it("prefers what the page set at runtime", () => {
    expect(pageTitle({ title: "Cross the Pond" }, { title: "Event" }, "/admin/planning/events/4821")).toBe(
      "Cross the Pond",
    );
  });

  it("falls back to what the route declares", () => {
    expect(pageTitle({}, { title: "Event" }, "/admin/planning/events/4821")).toBe("Event");
  });

  it("falls back to the nav item the path sits under", () => {
    expect(pageTitle({}, {}, "/ops/tmu")).toBe("TMU");
  });

  it("is undefined for a path under no nav item at all", () => {
    expect(pageTitle({}, {}, "/nowhere")).toBeUndefined();
  });

  // Two events must not share a name — that is what a prefix match on `itemForPath` alone would do,
  // and what favoriting a page stored before VATUSA/OIS#312.
  it("distinguishes two detail pages under one nav item", () => {
    const a = pageTitle({ title: "Cross the Pond" }, {}, "/admin/planning/events/4821");
    const b = pageTitle({ title: "Light the Night" }, {}, "/admin/planning/events/9137");
    expect(a).not.toBe(b);
  });
});

// VATUSA/OIS#537: advisory items in an event TMI package.
//
// `itemSummary`'s trailing `return` assumed ground stop, and `submit`'s final branch was a bare
// `else` that also assumed it — so adding a kind to the union silently made it a ground stop in two
// places. These pin the advisory case rather than the fallthrough.
import {describe, expect, it} from "vitest";

import {itemSummary} from "./tmi-packages";
import type {TmiPackageItem} from "@/lib/events";

const item = (kind: string, payload: Record<string, unknown>) =>
  ({id: "i1", kind, payload} as unknown as TmiPackageItem);

describe("itemSummary for an advisory (VATUSA/OIS#537)", () => {
  it("names the facility and document type", () => {
    const s = itemSummary(
      item("advisory", {
        facility: "DCC",
        kind: "reroute",
        valid_from: "2026-10-02T14:00:00Z",
        valid_to: "2026-10-02T18:00:00Z",
      }),
    );

    expect(s).toContain("DCC");
    expect(s).toContain("reroute");
  });

  it("shows the validity window", () => {
    const s = itemSummary(
      item("advisory", {
        facility: "DCC",
        kind: "gdp",
        valid_from: "2026-10-02T14:00:00Z",
        valid_to: "2026-10-02T18:00:00Z",
      }),
    );

    expect(s).toContain("–"); // a range, not one timestamp
    expect(s.match(/14|18/), "the window's hours appear").not.toBeNull();
  });

  /**
   * The regression this guards. A ground stop summarises as `airport · scope · until`; an advisory
   * must not be rendered through that branch, which would print "· all · UFN" for a document that
   * has neither.
   */
  it("is not summarised as a ground stop", () => {
    const s = itemSummary(item("advisory", {facility: "DCC", kind: "reroute"}));

    expect(s).not.toContain("UFN");
    expect(s).not.toContain("all");
  });

  /** And a real ground stop still summarises as one. */
  it("leaves the ground-stop summary alone", () => {
    expect(itemSummary(item("ground_stop", {airport: "KDCA"}))).toContain("UFN");
  });

  /** An advisory with no window still names itself rather than rendering a stray separator. */
  it("omits the window when there is none", () => {
    const s = itemSummary(item("advisory", {facility: "DCC", kind: "reroute"}));

    expect(s).toContain("DCC");
    expect(s.endsWith("·") || s.endsWith(" · ")).toBe(false);
  });
});

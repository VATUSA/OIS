import {describe, expect, it} from "vitest";

import {needsAirports, TEMPLATES} from "./templates";
import type {AtcWidget, TableWidget} from "./types";

const nas = () => {
  const t = TEMPLATES.find((x) => x.id === "nas-watch");
  if (!t) throw new Error("the NAS watch template is missing");
  return t;
};

describe("the NAS watch template (VATUSA/OIS#476)", () => {
  it("asks for no airports, because the NAS is not an airport list", () => {
    expect(nas().airports).toBe("national");
  });

  it("builds a board with no context at all", () => {
    const state = nas().build({ icaos: [] });
    expect(state.widgets.length).toBeGreaterThan(0);
  });

  /** A widget with no grid cell is built and then never rendered. */
  it("places every widget it builds", () => {
    const { widgets, layout } = nas().build({ icaos: [] });
    expect(layout.map((c) => c.i).sort()).toEqual(widgets.map((w) => w.id).sort());
  });

  it("opens ranked by exceedance, not alphabetically", () => {
    const demand = nas()
      .build({ icaos: [] })
      .widgets.find((w): w is TableWidget => w.kind === "table" && w.source === "nas-demand");

    expect(demand, "the overload ranking is the point of the board").toBeDefined();
    // Seeding the sort is what stops the board opening on a table the user must sort before it
    // answers anything. The same seed the add-widget menu uses, so the two agree.
    expect(demand?.sort).toEqual([{ id: "exceedance", desc: true }]);
  });

  it("ranks FCA pressure too", () => {
    const fcas = nas()
      .build({ icaos: [] })
      .widgets.find((w): w is TableWidget => w.kind === "table" && w.source === "nas-fca-pressure");
    expect(fcas?.sort).toEqual([{ id: "count", desc: true }]);
  });

  it("shows ATC for the whole country, not one facility", () => {
    const atc = nas()
      .build({ icaos: [] })
      .widgets.find((w): w is AtcWidget => w.kind === "atc");
    expect(atc?.facility).toEqual({ kind: "national" });
  });

  it("is the only national template, so the gate has exactly one thing to hide", () => {
    expect(TEMPLATES.filter((t) => t.airports === "national").map((t) => t.id)).toEqual([
      "nas-watch",
    ]);
  });
});

/**
 * The bug this guards was real and was fixed without a test: the library prompted on
 * `t.airports !== "none"`, so the NAS — which needs no airports — was asked for an ICAO, and the
 * board came out named `` · NAS Watch``. Reverting the fix left the whole suite green.
 *
 * Asserting `airports === "national"` (above) does not cover it. That pins what the template
 * *declares*; this pins what the caller *does* with the declaration, and the defect lived in the
 * gap between them.
 */
describe("which templates prompt for airports (VATUSA/OIS#476)", () => {
  it("does not ask the NAS for an airport", () => {
    expect(needsAirports(nas())).toBe(false);
  });

  it("asks only the modes that genuinely take airports", () => {
    // Written as the full mapping rather than a spot check, so a new mode has to be classified here
    // deliberately instead of defaulting into prompting.
    const byMode = Object.fromEntries(
      TEMPLATES.map((t) => [t.airports, needsAirports(t)]),
    );
    expect(byMode).toMatchObject({one: true, many: true});
    for (const mode of ["none", "facility", "national"] as const) {
      if (mode in byMode) expect(byMode[mode], `${mode} must not prompt`).toBe(false);
    }
  });

  it("every template that does not prompt can build with no airports at all", () => {
    for (const t of TEMPLATES.filter((x) => !needsAirports(x))) {
      expect(() => t.build({icaos: []}), `${t.id} needs airports it is never asked for`).not.toThrow();
    }
  });
});

import {describe, expect, it} from "vitest";

import {SCOPES, icaoRows, parseScopePrefix, tmiRow} from "./command-scopes";

describe("parseScopePrefix", () => {
  it("switches to a uniquely-prefixed scope and strips the prefix", () => {
    expect(parseScopePrefix("@tmi ZNY", SCOPES)).toEqual({ scope: "tmis", rest: "ZNY" });
    expect(parseScopePrefix("@AIRC ", SCOPES)).toEqual({ scope: "aircraft", rest: "" });
    expect(parseScopePrefix("@airport kden", SCOPES)).toEqual({ scope: "airports", rest: "kden" });
  });

  it("needs the trailing space, so a half-typed prefix stays in the query", () => {
    expect(parseScopePrefix("@tmi", SCOPES)).toBeNull();
  });

  it("ignores an ambiguous prefix", () => {
    // Aircraft and Airport data both start with "air".
    expect(parseScopePrefix("@air ", SCOPES)).toBeNull();
  });

  it("only matches scopes the user can see", () => {
    const noTmis = SCOPES.filter((s) => s.id !== "tmis");
    expect(parseScopePrefix("@tmi ", noTmis)).toBeNull();
  });

  it("leaves plain queries alone", () => {
    expect(parseScopePrefix("DAL123", SCOPES)).toBeNull();
    expect(parseScopePrefix("@ nothing", SCOPES)).toBeNull();
  });
});

describe("icaoRows", () => {
  it("gives every row the airport it is named after", () => {
    const rows = icaoRows("KDEN");
    expect(rows.map((r) => r.to)).toEqual([
      "/ops/airport",
      "/admin/planning/airport-configs",
      "/admin/planning/airport-surface",
    ]);
    // Regression (#311): the configs/surface rows used to navigate with no `?icao=`, landing the
    // user on an empty airport picker after they picked a row that named KDEN.
    for (const row of rows) {
      expect(row.label).toContain("KDEN");
      expect(row.search).toEqual({ icao: "KDEN" });
    }
  });

  it("offers nothing for a query that isn't ICAO-shaped", () => {
    expect(icaoRows("KD")).toEqual([]);
    expect(icaoRows("KDENVER")).toEqual([]);
    expect(icaoRows("")).toEqual([]);
  });
});

describe("tmiRow", () => {
  it("filters the restrictions tab to the TMI's facility", () => {
    // Regression (#311): every TMI row navigated to the same unfiltered restrictions tab, so after
    // picking one of up to 20 rows there was no sign of which one you picked.
    expect(tmiRow({ requesting: "ZNY" })).toEqual({
      to: "/ops/tmu",
      search: { tab: "restrictions", facility: "ZNY" },
    });
  });
});

describe("SCOPES", () => {
  it("gives every scope a sentence noun for the empty and loading copy", () => {
    // Regression (#311): the copy lower-cased the chip label, rendering "No tmis match."
    expect(SCOPES.find((s) => s.id === "tmis")?.noun).toBe("TMIs");
    for (const s of SCOPES) expect(s.noun).not.toBe("");
  });
});

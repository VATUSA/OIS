import {describe, expect, it} from "vitest";

import {formatRules, parseRules} from "./airport-configs";

describe("departure rule text (#512)", () => {
  it("round-trips a rule map through the editor's text form", () => {
    const rules = { CAMRN: "26R", HAPIE: "28" };
    expect(parseRules(formatRules(rules))).toEqual(rules);
  });

  it("upper-cases both halves, so case is never a different rule", () => {
    expect(parseRules("camrn=26r")).toEqual({ CAMRN: "26R" });
  });

  it("drops a fragment that names no runway rather than storing a rule pointing nowhere", () => {
    expect(parseRules("CAMRN=26R, JUNK, HAPIE=")).toEqual({ CAMRN: "26R" });
  });

  it("treats an empty field as no rules", () => {
    expect(parseRules("")).toEqual({});
    expect(formatRules(undefined)).toBe("");
    expect(formatRules(null)).toBe("");
  });
});

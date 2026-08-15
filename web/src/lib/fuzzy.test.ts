import {describe, expect, it} from "vitest";

import {fuzzyMatch, rankAircraft, type SearchableAircraft} from "./fuzzy";

describe("fuzzyMatch", () => {
  it("matches an exact prefix and reports positions", () => {
    const r = fuzzyMatch("AAL12", "AAL1234");
    expect(r).not.toBeNull();
    expect(r!.positions).toEqual([0, 1, 2, 3, 4]);
  });

  it("matches a non-contiguous subsequence", () => {
    expect(fuzzyMatch("AL4", "AAL1234")).not.toBeNull();
  });

  it("returns null when a character is missing", () => {
    expect(fuzzyMatch("AALX", "AAL1234")).toBeNull();
  });

  it("returns null when the query is longer than the text", () => {
    expect(fuzzyMatch("AAL12345", "AAL1234")).toBeNull();
  });

  it("is case-insensitive", () => {
    expect(fuzzyMatch("aal", "AAL1234")).not.toBeNull();
  });

  it("scores a contiguous prefix higher than a scattered match", () => {
    const prefix = fuzzyMatch("AAL", "AAL1234")!;
    const scattered = fuzzyMatch("AAL", "AXAXL999")!;
    expect(prefix.score).toBeGreaterThan(scattered.score);
  });

  it("treats an empty query as a trivial match", () => {
    expect(fuzzyMatch("", "AAL1234")).toEqual({score: 0, positions: []});
  });
});

describe("rankAircraft", () => {
  const fleet: SearchableAircraft[] = [
    {callsign: "AAL1234", dep: "KDFW", arr: "KLAX", actype: "B738"},
    {callsign: "AAL987", dep: "KJFK", arr: "KSFO", actype: "A321"},
    {callsign: "DAL55", dep: "KATL", arr: "KLAX", actype: "B739"},
    {callsign: "SWA4040", dep: "KDAL", arr: "KLAS", actype: "B737"},
  ];

  it("returns nothing for a blank query", () => {
    expect(rankAircraft("  ", fleet)).toEqual([]);
  });

  it("ranks the strongest callsign match first", () => {
    const hits = rankAircraft("AAL12", fleet);
    expect(hits[0].ac.callsign).toBe("AAL1234");
  });

  it("finds flights by destination airport", () => {
    const hits = rankAircraft("KLAX", fleet);
    const callsigns = hits.map((h) => h.ac.callsign);
    expect(callsigns).toContain("AAL1234");
    expect(callsigns).toContain("DAL55");
  });

  it("finds flights by aircraft type", () => {
    const hits = rankAircraft("A321", fleet);
    expect(hits[0].ac.callsign).toBe("AAL987");
  });

  it("honors the result limit", () => {
    expect(rankAircraft("A", fleet, 2).length).toBeLessThanOrEqual(2);
  });
});

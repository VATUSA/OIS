import {describe, expect, it} from "vitest";

import {fmtGap, nmAt, orderReason, orderReasonText, pairGapSec} from "@/lib/fca-compare";

// `ahead` is the lower sequence number; ETAs are ISO strings as the API sends them.
const f = (callsign: string, eta: string | null, manually_placed: boolean) => ({
  callsign,
  eta,
  manually_placed,
});
const EARLY = "2026-10-27T12:00:00Z";
const LATE = "2026-10-27T12:05:00Z";

describe("orderReason (VATUSA/OIS#819)", () => {
  it("returns manual when both flights were placed by the controller, whatever their ETAs", () => {
    expect(orderReason(f("A", EARLY, true), f("B", LATE, true))).toBe("manual");
    expect(orderReason(f("A", LATE, true), f("B", EARLY, true))).toBe("manual");
  });

  it("returns manual when the behind flight has the earlier ETA and the ahead flight was placed", () => {
    expect(orderReason(f("A", LATE, true), f("B", EARLY, false))).toBe("manual");
  });

  it("returns manual when the behind flight has the earlier ETA and only the behind flight was placed", () => {
    expect(orderReason(f("A", LATE, false), f("B", EARLY, true))).toBe("manual");
  });

  it("returns spacing when the behind flight has the earlier ETA and neither was placed", () => {
    expect(orderReason(f("A", LATE, false), f("B", EARLY, false))).toBe("spacing");
  });

  it("returns eta when the flights cross in ETA order and neither was placed", () => {
    expect(orderReason(f("A", EARLY, false), f("B", LATE, false))).toBe("eta");
  });

  it("returns eta when the flights cross in ETA order and only one was placed", () => {
    expect(orderReason(f("A", EARLY, true), f("B", LATE, false))).toBe("eta");
    expect(orderReason(f("A", EARLY, false), f("B", LATE, true))).toBe("eta");
  });

  it("returns eta when both ETAs are equal and neither was placed", () => {
    expect(orderReason(f("A", EARLY, false), f("B", EARLY, false))).toBe("eta");
  });

  it("returns eta when either ETA is missing and neither was placed", () => {
    expect(orderReason(f("A", null, false), f("B", EARLY, false))).toBe("eta");
    expect(orderReason(f("A", LATE, false), f("B", null, false))).toBe("eta");
  });
});

describe("orderReasonText (VATUSA/OIS#819)", () => {
  it("names both callsigns in one sentence for manual placement", () => {
    expect(orderReasonText("manual", "AAL1", "DAL2")).toBe(
      "AAL1 is ahead of DAL2 by manual placement.",
    );
  });

  it("names both callsigns in one sentence for a spacing push", () => {
    expect(orderReasonText("spacing", "AAL1", "DAL2")).toBe(
      "DAL2 has the earlier ETA but was pushed behind AAL1 to hold spacing.",
    );
  });

  it("names both callsigns in one sentence for ETA order", () => {
    expect(orderReasonText("eta", "AAL1", "DAL2")).toBe("AAL1 is ahead of DAL2 in ETA order.");
  });
});

describe("pairGapSec, fmtGap and nmAt (VATUSA/OIS#819)", () => {
  it("measures the gap from the ahead flight's crossing to the behind one's", () => {
    expect(
      pairGapSec({cross_time: "2026-10-27T12:00:00Z"}, {cross_time: "2026-10-27T12:02:30Z"}),
    ).toBe(150);
    expect(pairGapSec({cross_time: null}, {cross_time: "2026-10-27T12:02:30Z"})).toBeNull();
    expect(pairGapSec({cross_time: "2026-10-27T12:00:00Z"}, {cross_time: null})).toBeNull();
  });

  it("rounds a sub-second gap to the nearest second", () => {
    expect(
      pairGapSec({cross_time: "2026-10-27T12:00:00.000Z"}, {cross_time: "2026-10-27T12:02:29.600Z"}),
    ).toBe(150);
  });

  it("formats a gap as minutes:seconds and nm, keeping the sign of an out-of-order pair", () => {
    expect(fmtGap(150, 12.5)).toBe("2:30 · 12.5nm");
    expect(fmtGap(-65, -5)).toBe("-1:05 · -5.0nm");
  });

  it("converts seconds to nm at a speed, to one decimal", () => {
    // 150 s at 300 kt = 12.5 nm; 120 s at 283 kt = 9.433 nm.
    expect(nmAt(150, 300)).toBe(12.5);
    // 500 s at 283.4 kt = 39.36 nm, which rounds up.
    expect(nmAt(500, 283.4)).toBe(39.4);
    expect(nmAt(120, 283)).toBe(9.4);
  });
});

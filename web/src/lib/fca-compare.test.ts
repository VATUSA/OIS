import {describe, expect, it} from "vitest";

import {
  fmtGap,
  fmtMmss,
  nmAt,
  orderReason,
  orderReasonText,
  pairGapSec,
  shortfallSec,
} from "@/lib/fca-compare";

// `ahead` is the lower sequence number; ETAs are ISO strings as the API sends them.
const f = (callsign: string, eta: string | null, manually_placed: boolean, released = false) => ({
  callsign,
  eta,
  manually_placed,
  released,
});
const EARLY = "2026-10-27T12:00:00Z";
const LATE = "2026-10-27T12:05:00Z";

describe("orderReason (VATUSA/OIS#819)", () => {
  it("returns release whenever the behind flight holds an issued release, whatever else applies", () => {
    expect(orderReason(f("A", LATE, false), f("B", EARLY, false, true))).toBe("release");
    expect(orderReason(f("A", EARLY, false), f("B", LATE, false, true))).toBe("release");
    expect(orderReason(f("A", EARLY, true), f("B", LATE, true, true))).toBe("release");
  });

  it("does not return release when only the ahead flight is released", () => {
    expect(orderReason(f("A", LATE, false, true), f("B", EARLY, false))).toBe("spacing");
  });

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

  it("names the behind flight's issued release time for the release reason", () => {
    expect(orderReasonText("release", "AAL1", "DAL2")).toBe("DAL2 crosses at its issued release time.");
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

describe("shortfallSec (VATUSA/OIS#819)", () => {
  const g = (gap_to_leader_sec: number | null, required_spacing_sec: number) => ({
    gap_to_leader_sec,
    required_spacing_sec,
  });

  it("returns how far the gap falls short of the required spacing", () => {
    expect(shortfallSec(g(167, 327))).toBe(160);
  });

  it("counts a negative gap (a release crossing first) in full", () => {
    expect(shortfallSec(g(-30, 120))).toBe(150);
  });

  it("returns null for a gap equal to or above the required spacing", () => {
    expect(shortfallSec(g(120, 120))).toBeNull();
    expect(shortfallSec(g(121, 120))).toBeNull();
  });

  it("returns null for the first flight, which has no gap", () => {
    expect(shortfallSec(g(null, 120))).toBeNull();
  });
});

describe("fmtMmss (VATUSA/OIS#819)", () => {
  it("formats seconds as m:ss with a sign when negative", () => {
    expect(fmtMmss(160)).toBe("2:40");
    expect(fmtMmss(-65)).toBe("-1:05");
  });
});

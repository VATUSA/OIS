import {describe, expect, it} from "vitest";

import {type MonitorBin, type MonitorRow, alertedWithin, defaultOrder, moveRow, sliceBins} from "./monitor";

const NOW = Date.parse("2026-10-04T14:07:00Z");
const FIRST = Date.parse("2026-10-04T14:00:00Z");

/** Six hours of green bins, with `alerts` (bin index → level) laid over them. */
function row(id: string, alerts: Record<number, "amber" | "red"> = {}): MonitorRow {
  const bins: MonitorBin[] = Array.from({ length: 24 }, (_, i) => ({
    start: new Date(FIRST + i * 15 * 60_000).toISOString(),
    active: 0,
    proposed: 0,
    combined: 0,
    alert: alerts[i] ?? "green",
  }));
  return { sector_id: id, name: null, map: 10, consolidated: [], staffed: false, bins };
}

describe("sliceBins (AC3)", () => {
  it("slices the same six-hour payload for every Time Range", () => {
    const bins = row("10").bins;
    expect([2, 3, 4, 5, 6].map((h) => sliceBins(bins, h).length)).toEqual([8, 12, 16, 20, 24]);
    expect(bins).toHaveLength(24);
  });
});

describe("alertedWithin (AC4)", () => {
  // Bin 8 starts at 1600Z (2 h after the 1400Z first bin); bin 20 at 1900Z.
  it("is judged on the full payload, independent of the Time Range", () => {
    const late = row("20", { 20: "red" });
    expect(sliceBins(late.bins, 4).some((b) => b.alert !== "green")).toBe(false);
    expect(alertedWithin(late, 6, NOW)).toBe(true);
  });

  it("hides a row whose only alert is beyond the window", () => {
    const at1600 = row("30", { 8: "amber" });
    expect(alertedWithin(at1600, 1.5, NOW)).toBe(false);
    expect(alertedWithin(at1600, 2, NOW)).toBe(true);
  });

  it("counts the bin containing now, and shows everything at 0", () => {
    expect(alertedWithin(row("40", { 0: "red" }), 0.5, NOW)).toBe(true);
    expect(alertedWithin(row("50"), 0, NOW)).toBe(true);
    expect(alertedWithin(row("50"), 6, NOW)).toBe(false);
  });
});

describe("moveRow (AC5)", () => {
  it("steps past a row the filter hides", () => {
    const visible = new Set(["A", "C", "D"]);
    expect(moveRow(["A", "B", "C", "D"], "A", 1, visible)).toEqual(["B", "C", "A", "D"]);
    expect(moveRow(["A", "B", "C", "D"], "C", -1, visible)).toEqual(["C", "A", "B", "D"]);
  });

  it("does nothing off either end or for a hidden row", () => {
    const all = new Set(["A", "B"]);
    expect(moveRow(["A", "B"], "B", 1, all)).toEqual(["A", "B"]);
    expect(moveRow(["A", "B"], "A", -1, all)).toEqual(["A", "B"]);
    expect(moveRow(["A", "B"], "B", -1, new Set(["A"]))).toEqual(["A", "B"]);
  });
});

describe("defaultOrder (AC7)", () => {
  it("sorts numeric ids numerically, so 16 comes before 100", () => {
    expect(defaultOrder(["100", "16", "02", "9"])).toEqual(["02", "9", "16", "100"]);
  });

  it("falls back to a string order for non-numeric ids", () => {
    expect(defaultOrder(["B2", "A10", "A2"])).toEqual(["A10", "A2", "B2"]);
  });
});

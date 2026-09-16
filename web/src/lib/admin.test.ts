import {describe, expect, it} from "vitest";

import {weekOverWeek} from "./admin";

const series = (counts: number[]) => ({
  total: counts.reduce((a, b) => a + b, 0),
  points: counts.map((count, i) => ({ day: `2026-09-${String(i + 1).padStart(2, "0")}`, count })),
});

describe("weekOverWeek", () => {
  it("compares the last window with the one before it", () => {
    expect(weekOverWeek(series([1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2]))).toEqual({ direction: "up", text: "75.0% (+6)" });
    expect(weekOverWeek(series([2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1]))).toEqual({ direction: "down", text: "50.0% (-7)" });
  });

  it("is flat when nothing changed and 100% up from zero", () => {
    expect(weekOverWeek(series(new Array(14).fill(0)))).toEqual({ direction: "flat", text: "0.0% (+0)" });
    expect(weekOverWeek(series([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]))).toEqual({ direction: "up", text: "100.0% (+3)" });
  });
});

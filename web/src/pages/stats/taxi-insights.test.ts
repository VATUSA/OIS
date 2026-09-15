import {describe, expect, it} from "vitest";

import {fmtDur} from "./taxi-insights";

describe("fmtDur", () => {
  it("formats whole seconds", () => {
    expect(fmtDur(0)).toBe("0m 0s");
    expect(fmtDur(65)).toBe("1m 5s");
    expect(fmtDur(300)).toBe("5m 0s");
  });

  it("shows — for unknown values", () => {
    expect(fmtDur(null)).toBe("—");
    expect(fmtDur(undefined)).toBe("—");
  });

  // Regression: a fractional remainder rounding up to 60 must carry into the minute instead of
  // displaying e.g. "4m 60s" — reproducible with a real f64 median from an even sample count.
  it("carries a rounded-up seconds remainder into the minute", () => {
    expect(fmtDur(299.5)).toBe("5m 0s");
    expect(fmtDur(359.6)).toBe("6m 0s");
  });

  it("rounds an ordinary fractional remainder without carrying", () => {
    expect(fmtDur(304.4)).toBe("5m 4s");
  });
});

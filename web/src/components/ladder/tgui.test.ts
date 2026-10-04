import {describe, expect, it} from "vitest";

import {delayLabel, tickMarks} from "./tgui";

describe("delayLabel (VATUSA/OIS#557, the manual's §7.7 table)", () => {
  // Both sides of every band edge — a value just inside a band proves nothing about where it ends.
  it.each([
    [0.49, null],
    [0.5, { text: "01", level: "ok" }],
    [5, { text: "05", level: "ok" }],
    [5.49, { text: "05", level: "ok" }],
    [5.5, { text: "06", level: "watch" }],
    [6, { text: "06", level: "watch" }],
    [14, { text: "14", level: "watch" }],
    [15, { text: "15", level: "over" }],
    [99, { text: "99", level: "over" }],
    [99.49, { text: "99", level: "over" }],
    [99.5, { text: "++", level: "over" }],
    [100, { text: "++", level: "over" }],
    [0, null],
    [-0.49, null],
    [-0.5, null],
    [-0.51, { text: "-1", level: "early" }],
    [-1, { text: "-1", level: "early" }],
    [-12, { text: "-12", level: "early" }],
  ])("%s min → %j", (min, expected) => {
    expect(delayLabel(min)).toEqual(expected);
  });

  it("draws nothing when there is no delay to report, rather than inventing one", () => {
    expect(delayLabel(null)).toBeNull();
    expect(delayLabel(undefined)).toBeNull();
    expect(delayLabel(Number.NaN)).toBeNull();
  });
});

describe("tickMarks", () => {
  const now = Date.parse("2026-10-03T00:02:30Z");

  it("puts a tick on every clock minute in the window, starting at the next one", () => {
    const ticks = tickMarks(now, 10);
    expect(ticks).toHaveLength(10); // 00:03 … 00:12
    expect(ticks[0].min).toBeCloseTo(0.5);
    expect(ticks.every((t, i) => i === 0 || t.min - ticks[i - 1].min === 1)).toBe(true);
  });

  it("labels only the five-minute ticks, in Zulu HHMM between the rails", () => {
    const major = tickMarks(now, 10).filter((t) => t.major);
    expect(major.map((t) => t.label)).toEqual(["0005", "0010"]);
    expect(tickMarks(now, 10).filter((t) => !t.major).every((t) => t.label === null)).toBe(true);
  });

  it("includes a tick exactly at the window's end, and a tick at now itself", () => {
    const onTheMinute = Date.parse("2026-10-03T00:00:00Z");
    const ticks = tickMarks(onTheMinute, 5);
    expect(ticks[0]).toEqual({ min: 0, major: true, label: "0000" });
    expect(ticks.at(-1)).toEqual({ min: 5, major: true, label: "0005" });
  });
});

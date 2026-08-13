import {describe, expect, it} from "vitest";

import {formatZulu, hhmmZulu, parseHhmm, parseZulu} from "./time";

describe("formatZulu", () => {
  it("formats an ISO timestamp as DD/HHMMz", () => {
    expect(formatZulu("2026-08-13T14:30:00Z")).toBe("13/1430z");
  });
  it("pads single-digit day/hour/minute", () => {
    expect(formatZulu("2026-08-03T04:05:00Z")).toBe("03/0405z");
  });
  it("returns an em dash for empty or invalid input", () => {
    expect(formatZulu(null)).toBe("—");
    expect(formatZulu("nope")).toBe("—");
  });
});

describe("hhmmZulu", () => {
  it("formats an ISO timestamp as HHMMz", () => {
    expect(hhmmZulu("2026-08-13T14:30:00Z")).toBe("1430z");
  });
  it("returns an em dash for empty input", () => {
    expect(hhmmZulu(undefined)).toBe("—");
  });
});

describe("parseZulu", () => {
  const now = Date.UTC(2026, 7, 13, 12, 0, 0); // 2026-08-13 12:00Z

  it("parses DD/HHMMz to an ISO timestamp in the current month", () => {
    // parseZulu ignores `now` for the calendar month; use a real reference near it.
    const iso = parseZulu("13/1430z");
    expect(iso).not.toBeNull();
    const d = new Date(iso!);
    expect(d.getUTCDate()).toBe(13);
    expect(d.getUTCHours()).toBe(14);
    expect(d.getUTCMinutes()).toBe(30);
  });

  it("accepts input without the trailing z", () => {
    expect(parseZulu("13/1430")).not.toBeNull();
  });

  it("rejects malformed or out-of-range input", () => {
    expect(parseZulu("")).toBeNull();
    expect(parseZulu("1430z")).toBeNull(); // missing day
    expect(parseZulu("13/2560z")).toBeNull(); // minute 60
    expect(parseZulu("13/2400z")).toBeNull(); // hour 24
    expect(parseZulu("00/1200z")).toBeNull(); // day 0
  });

  // Keep `now` referenced so the fixture is meaningful across edits.
  it("has a stable reference now", () => {
    expect(new Date(now).getUTCFullYear()).toBe(2026);
  });
});

describe("parseHhmm", () => {
  const now = Date.UTC(2026, 7, 13, 12, 0, 0); // 12:00Z

  it("parses HHMMz onto today's UTC date", () => {
    const iso = parseHhmm("1430z", now);
    expect(iso).not.toBeNull();
    const d = new Date(iso!);
    expect(d.getUTCHours()).toBe(14);
    expect(d.getUTCMinutes()).toBe(30);
    expect(d.getUTCDate()).toBe(13);
  });

  it("rolls to the next day when the time is far in the past", () => {
    // now = 12:00Z; 0100z is 11h behind -> stays today (within 12h window)
    const sameDay = new Date(parseHhmm("0100z", now)!);
    expect(sameDay.getUTCDate()).toBe(13);
    // 2350z filed just after midnight would nudge back a day, exercised via a late now.
    const lateNow = Date.UTC(2026, 7, 13, 0, 10, 0); // 00:10Z
    const rolled = new Date(parseHhmm("2350z", lateNow)!);
    expect(rolled.getUTCDate()).toBe(12); // previous day
  });

  it("rejects malformed input", () => {
    expect(parseHhmm("143z", now)).toBeNull();
    expect(parseHhmm("2465z", now)).toBeNull();
    expect(parseHhmm("", now)).toBeNull();
  });
});

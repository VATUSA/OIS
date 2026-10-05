import {describe, expect, it} from "vitest";

import {parseLimit, usageLabel} from "./rate-limits";

describe("parseLimit", () => {
  it("sets a positive whole number", () => {
    expect(parseLimit("600")).toBe(600);
    expect(parseLimit(" 30 ")).toBe(30);
  });

  it("clears on empty input", () => {
    expect(parseLimit("")).toBeNull();
    expect(parseLimit("   ")).toBeNull();
  });

  it.each(["0", "-5", "1.5", "abc", "1e3"])("sends nothing for %j", (input) => {
    expect(parseLimit(input)).toBeUndefined();
  });
});

describe("usageLabel", () => {
  it("shows the hour, the day and any refusals", () => {
    expect(usageLabel({requests_this_hour: 120, requests_last_day: 2400, refused_last_day: 3})).toBe(
      "120 this hour · 2,400 / 24 h · 3 refused",
    );
    expect(usageLabel({requests_this_hour: 0, requests_last_day: 0, refused_last_day: 0})).toBe(
      "0 this hour · 0 / 24 h",
    );
  });
});

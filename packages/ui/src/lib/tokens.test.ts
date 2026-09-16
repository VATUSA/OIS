import {describe, expect, it} from "vitest";

import {parseColor} from "./tokens";

describe("parseColor", () => {
  it("parses #rrggbb and #rgb", () => {
    expect(parseColor("#43d089")).toEqual([67, 208, 137, 255]);
    expect(parseColor("#fff")).toEqual([255, 255, 255, 255]);
  });

  it("parses space and comma rgb() forms with alpha", () => {
    expect(parseColor("rgb(10 12 16 / 0.71)")).toEqual([10, 12, 16, 181]);
    expect(parseColor("rgba(251, 107, 107, 0.13)")).toEqual([251, 107, 107, 33]);
    expect(parseColor("rgb(1, 2, 3)")).toEqual([1, 2, 3, 255]);
  });

  it("falls back to opaque grey for an unknown value", () => {
    expect(parseColor("")).toEqual([128, 128, 128, 255]);
    expect(parseColor("var(--nope)")).toEqual([128, 128, 128, 255]);
  });
});

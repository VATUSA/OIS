import {describe, expect, it} from "vitest";

import {parseColor} from "./tokens";

describe("parseColor", () => {
  it("parses #rrggbb and #rgb", () => {
    expect(parseColor("#43d089")).toEqual([67, 208, 137, 255]);
    expect(parseColor("#fff")).toEqual([255, 255, 255, 255]);
  });

  // The production CSS minifier rewrites `rgb(r g b / a)` tokens to 8-digit hex; these are the
  // exact values the built stylesheet ships for the map tokens.
  it("parses the #rrggbbaa and #rgba forms the minifier emits", () => {
    expect(parseColor("#828ca06e")).toEqual([130, 140, 160, 110]);
    expect(parseColor("#0a0c10b5")).toEqual([10, 12, 16, 181]);
    expect(parseColor("#FFFFFFBF")).toEqual([255, 255, 255, 191]);
    expect(parseColor("#f008")).toEqual([255, 0, 0, 136]);
  });

  it("rejects hex of any other length", () => {
    expect(parseColor("#12345")).toEqual([128, 128, 128, 255]);
    expect(parseColor("#123456789")).toEqual([128, 128, 128, 255]);
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

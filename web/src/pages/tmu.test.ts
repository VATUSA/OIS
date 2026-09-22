import {describe, expect, it} from "vitest";

import {tmuTitle} from "./tmu";

describe("tmuTitle (VATUSA/OIS#339)", () => {
  // The title is the favorite's label and `?tab=` is in its key: one favorite per tab has to be
  // told apart by name, not by a column of identical "TMU" rows.
  it("names the active tab", () => {
    expect(tmuTitle("Ground delay")).toBe("TMU · Ground delay");
    expect(tmuTitle("Restrictions")).not.toBe(tmuTitle("Ground delay"));
  });

  it("is plain TMU when no tab is showing", () => {
    expect(tmuTitle(undefined)).toBe("TMU");
  });
});

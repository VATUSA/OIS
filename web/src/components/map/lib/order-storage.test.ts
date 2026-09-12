import {describe, expect, it} from "vitest";

import {applyOrder} from "./order-storage";

describe("applyOrder", () => {
  it("returns the natural order when nothing has been ordered yet", () => {
    expect(applyOrder(["a", "b", "c"], [])).toEqual(["a", "b", "c"]);
  });

  it("applies the stored order for known ids", () => {
    expect(applyOrder(["a", "b", "c"], ["c", "a", "b"])).toEqual(["c", "a", "b"]);
  });

  it("appends a new id (not yet in the stored order) at the end", () => {
    expect(applyOrder(["a", "b", "c"], ["b", "a"])).toEqual(["b", "a", "c"]);
  });

  it("drops stored ids that no longer exist (deleted items)", () => {
    expect(applyOrder(["a", "c"], ["c", "b", "a"])).toEqual(["c", "a"]);
  });

  it("handles an empty live list", () => {
    expect(applyOrder([], ["a", "b"])).toEqual([]);
  });
});

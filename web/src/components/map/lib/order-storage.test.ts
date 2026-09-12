import {arrayMove} from "@dnd-kit/sortable";
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

describe("reordering under a narrowed (filtered) view", () => {
  it("a drag resolved against the full order's indices never drops items outside the filter (see #109 QA)", () => {
    const allIds = ["a", "b", "c", "d", "e"];
    let order = applyOrder(allIds, []); // establish the natural order first

    // Simulate a filtered view showing only ["a", "b"] and a drag that swaps them. The fix: look
    // up the drag's from/to indices in the FULL order (order.indexOf), never in the filtered
    // subset — that's what makes this safe regardless of which items the current filter hides.
    const from = order.indexOf("a");
    const to = order.indexOf("b");
    order = arrayMove(order, from, to);

    // Every id survives the drag, including the ones the filter was hiding.
    expect(order).toHaveLength(5);
    expect(order).toEqual(expect.arrayContaining(["c", "d", "e"]));
    expect(order.indexOf("b")).toBeLessThan(order.indexOf("a"));
  });
});

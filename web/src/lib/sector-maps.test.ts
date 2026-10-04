import {describe, expect, it} from "vitest";

import {mapEdit} from "./sector-maps";

describe("mapEdit", () => {
  it("writes a positive whole number that differs from the current value", () => {
    expect(mapEdit("14", 10)).toBe(14);
    expect(mapEdit(" 7 ", 10)).toBe(7);
    expect(mapEdit("10", 14)).toBe(10); // typing the default is the reset
  });

  it.each(["", "   ", "0", "-3", "1.5", "abc", "1e2"])("writes nothing for %j", (input) => {
    expect(mapEdit(input, 10)).toBeNull();
  });

  it("writes nothing when the value is unchanged", () => {
    expect(mapEdit("10", 10)).toBeNull();
    expect(mapEdit("014", 14)).toBeNull();
  });
});

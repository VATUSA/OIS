import {describe, expect, it} from "vitest";

import {preferencesFrom} from "./preferences";

describe("preferencesFrom", () => {
  it("returns the stored value on a successful load", () => {
    expect(preferencesFrom<{ lastSeenId: string }>(true, { lastSeenId: "x" })).toEqual({ lastSeenId: "x" });
  });

  // The API answers an unset namespace with `{}` — that's a successful, empty load.
  it("treats an unset namespace as a successful empty value", () => {
    expect(preferencesFrom(true, {})).toEqual({});
    expect(preferencesFrom(true, undefined)).toBeNull();
  });

  // A failed load must be an error, never `null`, or writers overwrite what's stored.
  it("throws on a failed load, with or without an error body", () => {
    expect(() => preferencesFrom(false, undefined)).toThrow();
    expect(() => preferencesFrom(false, { lastSeenId: "x" })).toThrow();
  });
});

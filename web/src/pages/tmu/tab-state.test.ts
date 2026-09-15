import {describe, expect, it} from "vitest";

import {resolveTmuTab, shouldSaveLastTmuTab} from "./tab-state";

const allTabs = [
  { id: "programs" as const },
  { id: "restrictions" as const },
  { id: "ground-stops" as const },
];

describe("resolveTmuTab", () => {
  it("defaults to the first visible tab when no tab is requested, and leaves the URL alone", () => {
    expect(resolveTmuTab(allTabs, undefined)).toEqual({
      active: "programs",
      needsUrlSync: false,
    });
  });

  it("uses the requested tab when the user can see it, and leaves the URL alone", () => {
    expect(resolveTmuTab(allTabs, "restrictions")).toEqual({
      active: "restrictions",
      needsUrlSync: false,
    });
  });

  it("falls back to the first visible tab when the requested one isn't visible, and flags the URL for a rewrite (#247)", () => {
    // e.g. /ops/tmu?tab=gdp without tmu.gdp.read
    expect(resolveTmuTab(allTabs, "gdp")).toEqual({
      active: "programs",
      needsUrlSync: true,
    });
  });

  it("has no active tab when the user can see nothing, but still flags the stale URL for a rewrite", () => {
    expect(resolveTmuTab([], "programs")).toEqual({
      active: undefined,
      needsUrlSync: true,
    });
  });
});

describe("shouldSaveLastTmuTab", () => {
  it("saves a bare-URL default (no fallback involved)", () => {
    expect(shouldSaveLastTmuTab({ active: "programs", needsUrlSync: false })).toBe(true);
  });

  it("saves a validly requested tab", () => {
    expect(shouldSaveLastTmuTab({ active: "restrictions", needsUrlSync: false })).toBe(true);
  });

  /** Regression (#247 rework): an involuntary permission-fallback must never overwrite the
   * user's real "last viewed" preference with whatever tab they got bounced to. */
  it("does not save an involuntary permission-fallback tab", () => {
    expect(shouldSaveLastTmuTab({ active: "programs", needsUrlSync: true })).toBe(false);
  });

  it("does not save when there's no active tab at all", () => {
    expect(shouldSaveLastTmuTab({ active: undefined, needsUrlSync: true })).toBe(false);
  });
});

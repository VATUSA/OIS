import {describe, expect, it} from "vitest";

import {resolveTmuTab} from "./tab-state";

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

  it("has no active tab and no URL sync when the user can see nothing", () => {
    expect(resolveTmuTab([], "programs")).toEqual({
      active: undefined,
      needsUrlSync: true,
    });
  });
});

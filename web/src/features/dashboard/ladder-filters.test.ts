import {describe, expect, it} from "vitest";

import {activeLadderFilters, ladderStatuses} from "./types";

// #814 decision 34: a ladder widget saved with the retired `proposed` status neither hides every
// flight nor shows an active filter.
describe("saved ladder filters", () => {
  it("keeps only the statuses the ladder offers", () => {
    expect(ladderStatuses({ statuses: ["proposed"] })).toEqual([]);
    expect(ladderStatuses({ statuses: ["ground", "proposed"] })).toEqual(["ground"]);
    expect(ladderStatuses(undefined)).toEqual([]);
  });

  it("counts a category only when it still narrows the ladder", () => {
    expect(activeLadderFilters({ statuses: ["proposed"] })).toBe(0);
    expect(activeLadderFilters({ statuses: ["airborne"] }), "positive control").toBe(1);
    expect(activeLadderFilters({ gates: ["ROBUC"], statuses: ["proposed"], types: ["B73"] })).toBe(2);
    expect(activeLadderFilters(undefined)).toBe(0);
  });
});

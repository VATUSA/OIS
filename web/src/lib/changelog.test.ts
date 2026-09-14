import {describe, expect, it} from "vitest";

import {unseenEntries, type ChangelogEntry} from "./changelog";

const entries: ChangelogEntry[] = [
  { id: "c", date: "2026-03-01", title: "Third", highlights: [] },
  { id: "b", date: "2026-02-01", title: "Second", highlights: [] },
  { id: "a", date: "2026-01-01", title: "First", highlights: [] },
];

describe("unseenEntries", () => {
  it("returns everything newer than the given id", () => {
    expect(unseenEntries(entries, "b")).toEqual([entries[0]]);
  });

  it("returns nothing when the newest entry is already seen", () => {
    expect(unseenEntries(entries, "c")).toEqual([]);
  });

  it("returns everything when lastSeenId is undefined (brand-new/never-seeded)", () => {
    expect(unseenEntries(entries, undefined)).toEqual(entries);
  });

  it("returns everything when lastSeenId is older than anything still in the module", () => {
    expect(unseenEntries(entries, "long-since-trimmed")).toEqual(entries);
  });
});

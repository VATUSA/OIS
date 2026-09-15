import {describe, expect, it} from "vitest";

import {shouldSeed, unseenEntries, type ChangelogEntry} from "./changelog";

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

describe("shouldSeed", () => {
  // Regression (#206): `GET /api/v1/me/preferences/{namespace}` returns `{}`, not `null`, for an
  // unset namespace — a `prefs == null` check never fires, so every user (brand-new or existing)
  // fell into the "show unseen backlog" branch instead of being seeded silently. This is the exact
  // fix that already regressed once; it must check the field, not the blob.
  it("is true for an undefined blob (query still settling elsewhere, or an error)", () => {
    expect(shouldSeed(undefined)).toBe(true);
  });

  it("is true for a null blob", () => {
    expect(shouldSeed(null)).toBe(true);
  });

  it("is true for an empty blob — the actual shape a brand-new user's prefs resolve to", () => {
    expect(shouldSeed({})).toBe(true);
  });

  it("is false once lastSeenId is set", () => {
    expect(shouldSeed({ lastSeenId: "a" })).toBe(false);
  });
});

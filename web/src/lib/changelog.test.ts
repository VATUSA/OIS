import {describe, expect, it} from "vitest";

import {
  CHANGELOG,
  changelogProblems,
  panelSize,
  shotColumns,
  shouldSeed,
  unseenEntries,
  type ChangelogEntry,
  type Screenshot,
} from "./changelog";

const entries: ChangelogEntry[] = [
  { id: "c", date: "2026-03-01", title: "Third", sections: [] },
  { id: "b", date: "2026-02-01", title: "Second", sections: [] },
  { id: "a", date: "2026-01-01", title: "First", sections: [] },
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

const shot = (n: number, alt = `Shot ${n}`): Screenshot => ({ src: `/assets/shot-${n}.png`, alt });
const shots = (count: number) => Array.from({ length: count }, (_, i) => shot(i));
const entry = (id: string, sectionShots: Screenshot[][]): ChangelogEntry => ({
  id,
  date: "2026-10-01",
  title: id,
  sections: sectionShots.map((s) => ({ highlights: ["x"], shots: s })),
});

describe("shotColumns (#665)", () => {
  it.each([
    [1, 1],
    [2, 2],
    [3, 2],
    [4, 2],
    [5, 3],
    [8, 3],
  ])("lays %i shots out in %i columns", (count, cols) => {
    expect(shotColumns(count)).toBe(cols);
  });
});

describe("panelSize (#665)", () => {
  it("widens only when a shown entry has shots", () => {
    expect(panelSize([entry("a", [[]])])).toBe("md");
    expect(panelSize([entry("a", [[]]), entry("b", [[], shots(1)])])).toBe("xl");
  });
});

describe("changelogProblems (#665)", () => {
  it("passes the changelog that ships", () => {
    expect(changelogProblems(CHANGELOG)).toEqual([]);
  });

  it("caps shots per entry, summed across sections", () => {
    expect(changelogProblems([entry("a", [shots(8)])])).toEqual([]);
    expect(changelogProblems([entry("a", [shots(9)])])).toHaveLength(1);
    expect(changelogProblems([entry("a", [shots(5), shots(4)])])).toHaveLength(1);
  });

  it("allows shots only on the newest three entries", () => {
    const four = ["a", "b", "c", "d"].map((id) => entry(id, [[]]));
    four[2] = entry("c", [shots(1)]);
    expect(changelogProblems(four)).toEqual([]);
    four[3] = entry("d", [shots(1)]);
    expect(changelogProblems(four)).toEqual(["d: only the newest 3 entries may carry shots"]);
  });

  it("requires alt text", () => {
    expect(changelogProblems([entry("a", [[shot(1, "  ")]])])).toHaveLength(1);
  });

  it.each(["https://example.com/a.png", "http://example.com/a.png", "//cdn.example.com/a.png"])(
    "rejects a remote src (%s)",
    (src) => {
      expect(changelogProblems([entry("a", [[{ src, alt: "x" }]])])).toHaveLength(1);
    },
  );
});

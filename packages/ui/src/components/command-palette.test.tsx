import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {CommandRow, ScopeChips, activeIndex, cycleScope} from "./command-palette";

describe("cycleScope", () => {
  const ids = ["all", "aircraft", "tmis"];

  it("steps forward and back, wrapping at both ends", () => {
    expect(cycleScope(ids, "all", 1)).toBe("aircraft");
    expect(cycleScope(ids, "tmis", 1)).toBe("all");
    expect(cycleScope(ids, "all", -1)).toBe("tmis");
    expect(cycleScope(ids, "aircraft", -1)).toBe("all");
  });

  it("treats an unknown current scope as the first", () => {
    expect(cycleScope(ids, "gone", 1)).toBe("aircraft");
  });
});

describe("ScopeChips", () => {
  const scopes = [
    { id: "all", label: "All" },
    { id: "tmis", label: "TMIs" },
  ];

  it("marks only the active scope as checked", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('aria-checked="true"[^>]*>TMIs<'));
    expect(html).toMatch(new RegExp('aria-checked="false"[^>]*>All<'));
  });

  // The search field normally holds focus, so without these a screen-reader user can neither reach
  // the group nor hear the scope change (VATUSA/OIS#311).
  it("makes only the checked chip tabbable, so the group is one tab stop", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('tabindex="0"[^>]*>TMIs<'));
    expect(html).toMatch(new RegExp('tabindex="-1"[^>]*>All<'));
  });

  it("announces the active scope in a live region", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('aria-live="polite"[^>]*>TMIs scope<'));
  });
});

describe("activeIndex", () => {
  const rows = [{ id: "a" }, { id: "b" }, { id: "c" }];

  it("falls back to the first row when nothing is highlighted yet", () => {
    expect(activeIndex(rows, null)).toBe(0);
  });

  it("finds the highlighted row", () => {
    expect(activeIndex(rows, "b")).toBe(1);
  });

  it("falls back to the first row when nothing is remembered and the highlighted one is gone", () => {
    expect(activeIndex(rows, "vanished")).toBe(0);
    expect(activeIndex([], "b")).toBe(0);
  });

  // VATUSA/OIS#312: a row can vanish with no user input at all — traffic refetches every 15s — and
  // collapsing to 0 pointed the next Enter at a flight nobody picked.
  it("holds position when the highlighted row is gone, clamped to the list", () => {
    expect(activeIndex(rows, "vanished", { index: 2 })).toBe(2);
    expect(activeIndex([{ id: "a" }], "vanished", { index: 2 })).toBe(0);
    expect(activeIndex(rows, "vanished", { index: -5 })).toBe(0);
  });

  // ⌘⇧F is a toggle, and the reflex after one press is another. Un-starring drops the pinned
  // Favorites row, so the highlight has to find the source row it was starred from — otherwise the
  // undo press destroys whatever slid underneath instead.
  it("moves to the row standing for the same thing when the highlighted one is un-starred", () => {
    const after = [
      { id: "favorite:page:/ops/tmu", entityId: "page:/ops/tmu" },
      { id: "page:/ops/advisories", entityId: "page:/ops/advisories" },
      { id: "page:/ops/tmu", entityId: "page:/ops/tmu" },
      { id: "page:/ops/airport", entityId: "page:/ops/airport" },
    ];
    const gone = { index: 1, entityId: "page:/ops/airport" };
    expect(activeIndex(after, "favorite:page:/ops/airport", gone)).toBe(3);
  });

  it("prefers the exact row over its twin while both are present", () => {
    const rows2 = [
      { id: "favorite:page:/ops/tmu", entityId: "page:/ops/tmu" },
      { id: "page:/ops/tmu", entityId: "page:/ops/tmu" },
    ];
    expect(activeIndex(rows2, "page:/ops/tmu", { index: 0, entityId: "page:/ops/tmu" })).toBe(1);
  });

  // VATUSA/OIS#312: starring prepends a row to the pinned Favorites group. Holding a raw index here
  // slid the highlight to the row above, so an undo starred a second row and Enter opened the wrong
  // one. The highlight must follow the *item* across the rebuild.
  it("keeps the highlight on the same row when a favorite is prepended", () => {
    const before = [{ id: "page:/ops/advisories" }, { id: "page:/ops/tmu" }];
    const highlighted = before[1].id;
    expect(activeIndex(before, highlighted)).toBe(1);

    const afterStarring = [{ id: "favorite:page:/ops/tmu" }, ...before];
    expect(activeIndex(afterStarring, highlighted)).toBe(2);
    expect(afterStarring[activeIndex(afterStarring, highlighted)].id).toBe("page:/ops/tmu");
  });
});

describe("CommandRow", () => {
  const item = { id: "r1", label: "KDEN airport", onSelect: () => {} };
  const row = (over: Partial<React.ComponentProps<typeof CommandRow>> = {}) =>
    renderToStaticMarkup(
      <CommandRow item={item} index={0} active={false} onActivate={() => {}} onSelect={() => {}} {...over} />,
    );
  const starrable = { ...item, onToggleStar: () => {} };

  it("shows no star for a row that can't be favorited", () => {
    expect(row({ active: true })).not.toMatch(/favorites/);
  });

  it("keeps a starred row's star while the highlight is elsewhere", () => {
    expect(row({ item: { ...starrable, starred: true }, index: 3 })).toMatch('aria-label="Remove from favorites"');
  });

  it("offers the star on the highlighted row even when it isn't a favorite", () => {
    expect(row({ item: { ...starrable, starred: false }, active: true })).toMatch('aria-label="Add to favorites"');
  });

  it("hides the star on a row that is neither starred nor highlighted", () => {
    expect(row({ item: { ...starrable, starred: false }, index: 2 })).not.toMatch(/favorites/);
  });

  it("reports the favorite state through aria-pressed", () => {
    expect(row({ item: { ...starrable, starred: true }, active: true })).toMatch('aria-pressed="true"');
    expect(row({ item: { ...starrable, starred: false }, active: true })).toMatch('aria-pressed="false"');
  });

  // The star used to be a `span role="button"` inside the row button: a nested interactive role has
  // no defined accessibility behaviour, and it was why a star click needed stopPropagation to avoid
  // navigating (VATUSA/OIS#336).
  it("renders the star as a sibling of the row button, so its click can't reach the row", () => {
    const html = row({ item: { ...starrable, starred: true } });
    const rowButtonEnd = html.indexOf("</button>");
    expect(html.indexOf("aria-pressed")).toBeGreaterThan(rowButtonEnd);
    expect(html.slice(0, rowButtonEnd).match(/<button/g)).toHaveLength(1);
    expect(html.match(/<button/g)).toHaveLength(2);
    expect(html).not.toContain('role="button"');
  });

  it("keeps the Enter hint inside the selection button, so the whole row bar the star selects", () => {
    const html = row({ item: { ...starrable, starred: true }, active: true });
    expect(html.indexOf("lucide-corner-down-left")).toBeLessThan(html.indexOf("</button>"));
    expect(html.indexOf("lucide-star")).toBeGreaterThan(html.indexOf("</button>"));
  });
});

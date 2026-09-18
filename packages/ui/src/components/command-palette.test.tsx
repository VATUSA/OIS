import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {CommandRow, ScopeChips, activeIndex, cycleScope, isFavoriteHotkey} from "./command-palette";

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
    expect(activeIndex(rows, { id: "b" })).toBe(1);
  });

  // VATUSA/OIS#312: starring prepends a row to the pinned Favorites group. Holding a raw index here
  // slid the highlight to the row above, so an undo starred a second row and Enter opened the wrong
  // one. The highlight must follow the *item* across the rebuild.
  it("keeps the highlight on the same row when a favorite is prepended", () => {
    const before = [{ id: "page:/ops/advisories" }, { id: "page:/ops/tmu" }];
    const highlighted = { id: "page:/ops/tmu" };
    expect(activeIndex(before, highlighted)).toBe(1);

    const afterStarring = [{ id: "favorite:page:/ops/tmu" }, ...before];
    expect(afterStarring[activeIndex(afterStarring, highlighted)].id).toBe("page:/ops/tmu");
  });

  // Un-starring a pinned favorite removes the very row the highlight is on. Landing on the same
  // thing's row in its own group is what makes a second ⌘⇧F undo the first rather than hit a
  // neighbour.
  it("follows the entity when the highlighted row is gone", () => {
    const after = [{ id: "favorite:page:/ops/tmu", entity: "page:/ops/tmu" }, { id: "page:/ops/advisories", entity: "page:/ops/advisories" }, { id: "page:/ops/tmu", entity: "page:/ops/tmu" }];
    const highlighted = { id: "favorite:page:/ops/airport", entity: "page:/ops/airport" };
    // Airport is gone entirely — nothing to follow, so it holds its position instead of jumping.
    expect(activeIndex(after, highlighted, 2)).toBe(2);
    // TMU's pinned row is gone but its Pages row remains: follow it.
    expect(activeIndex(after.slice(1), { id: "favorite:page:/ops/tmu", entity: "page:/ops/tmu" }, 0)).toBe(1);
  });

  it("holds the position it had when the row vanishes, clamped to the list", () => {
    expect(activeIndex(rows, { id: "vanished" }, 2)).toBe(2);
    expect(activeIndex(rows, { id: "vanished" }, 9)).toBe(2);
    expect(activeIndex(rows, { id: "vanished" }, -1)).toBe(0);
    expect(activeIndex([], { id: "b" }, 3)).toBe(0);
  });

  it("without a remembered position, a vanished row still falls back to the first", () => {
    expect(activeIndex(rows, { id: "vanished" })).toBe(0);
  });
});

describe("isFavoriteHotkey", () => {
  const key = (over: Partial<KeyboardEvent>) =>
    ({ metaKey: false, ctrlKey: false, shiftKey: false, key: "f", ...over }) as KeyboardEvent;

  it("matches ⌘⇧F and Ctrl+⇧F", () => {
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true }))).toBe(true);
    expect(isFavoriteHotkey(key({ ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true, key: "F" }))).toBe(true);
  });

  it("leaves browser find (⌘F) and find-next (⌘G) alone", () => {
    expect(isFavoriteHotkey(key({ metaKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({ ctrlKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true, key: "g" }))).toBe(false);
  });

  it("ignores a bare ⇧F and a bare f", () => {
    expect(isFavoriteHotkey(key({ shiftKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({}))).toBe(false);
  });
});

describe("CommandRow", () => {
  const item = { id: "r1", label: "KDEN airport", onSelect: () => {} };

  it("shows no star for a row that can't be favorited", () => {
    const html = renderToStaticMarkup(<CommandRow item={item} index={0} active />);
    expect(html).not.toMatch(/favorites/);
  });

  it("keeps a starred row's star while the highlight is elsewhere", () => {
    const html = renderToStaticMarkup(
      <CommandRow item={{ ...item, starred: true, onToggleStar: () => {} }} index={3} active={false} />,
    );
    expect(html).toMatch('aria-label="Remove from favorites"');
  });

  it("offers the star on the highlighted row even when it isn't a favorite", () => {
    const html = renderToStaticMarkup(
      <CommandRow item={{ ...item, starred: false, onToggleStar: () => {} }} index={0} active />,
    );
    expect(html).toMatch('aria-label="Add to favorites"');
  });

  it("hides the star on a row that is neither starred nor highlighted", () => {
    const html = renderToStaticMarkup(
      <CommandRow item={{ ...item, starred: false, onToggleStar: () => {} }} index={2} active={false} />,
    );
    expect(html).not.toMatch(/favorites/);
  });

  it("reports the favorite state through aria-pressed", () => {
    const starred = renderToStaticMarkup(
      <CommandRow item={{ ...item, starred: true, onToggleStar: () => {} }} index={0} active />,
    );
    const unstarred = renderToStaticMarkup(
      <CommandRow item={{ ...item, starred: false, onToggleStar: () => {} }} index={0} active />,
    );
    expect(starred).toMatch('aria-pressed="true"');
    expect(unstarred).toMatch('aria-pressed="false"');
  });
});

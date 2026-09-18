import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {CommandRow, ScopeChips, cycleScope} from "./command-palette";

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

// The star used to render as a `span role="button"` *inside* the row button. `role="button"` nested
// in `role=button` is not a valid accessibility tree, so assistive tech could expose one control,
// the wrong one, or neither (VATUSA/OIS#336). These pin the two controls apart.
describe("CommandRow", () => {
  const item = { id: "kdca", label: "KDCA", onSelect: () => {} };
  const row = (props: Partial<React.ComponentProps<typeof CommandRow>> = {}) =>
    renderToStaticMarkup(
      <CommandRow item={item} index={0} active={false} onActivate={() => {}} onSelect={() => {}} {...props} />,
    );

  it("renders the star as a sibling of the row button, not inside it", () => {
    const html = row({ item: { ...item, starred: true, onToggleStar: () => {} } });
    const rowButtonEnd = html.indexOf("</button>");
    expect(rowButtonEnd).toBeGreaterThan(-1);
    expect(html.indexOf("aria-pressed")).toBeGreaterThan(rowButtonEnd);
    // …and the row button's own markup holds no second control (only its own opening tag).
    expect(html.slice(0, rowButtonEnd).match(/<button/g)).toHaveLength(1);
  });

  it("exposes both controls as real buttons, with no nested role", () => {
    const html = row({ active: true, item: { ...item, onToggleStar: () => {} } });
    expect(html.match(/<button/g)).toHaveLength(2);
    expect(html).not.toContain('role="button"');
  });

  // Splitting the row button must not carve the Enter hint out of the click target: the whole row
  // bar the star still selects, as it did when the row was one button.
  it("keeps the Enter hint inside the selection button", () => {
    const html = row({ active: true, item: { ...item, starred: true, onToggleStar: () => {} } });
    expect(html.indexOf("lucide-corner-down-left")).toBeLessThan(html.indexOf("</button>"));
    expect(html.indexOf("lucide-star")).toBeGreaterThan(html.indexOf("</button>"));
  });

  it("labels the star by what activating it will do", () => {
    expect(row({ item: { ...item, starred: true, onToggleStar: () => {} } })).toContain(
      'aria-label="Remove from favorites"',
    );
    expect(row({ active: true, item: { ...item, onToggleStar: () => {} } })).toContain(
      'aria-label="Add to favorites"',
    );
  });

  // A toggle button's state *is* `aria-pressed` — asserting only that the attribute exists would let
  // it be hard-coded, which is the one thing this issue is about.
  it("reports the favorite state through aria-pressed", () => {
    expect(row({ item: { ...item, starred: true, onToggleStar: () => {} } })).toContain('aria-pressed="true"');
    expect(row({ active: true, item: { ...item, starred: false, onToggleStar: () => {} } })).toContain(
      'aria-pressed="false"',
    );
  });

  it("shows the star on a starred row and on the active row, and nowhere else", () => {
    const starrable = { ...item, onToggleStar: () => {} };
    expect(row({ item: { ...starrable, starred: true } })).toContain("aria-pressed");
    expect(row({ item: starrable, active: true })).toContain("aria-pressed");
    expect(row({ item: starrable })).not.toContain("aria-pressed");
    // An item the caller never made favoritable stays starless even when active.
    expect(row({ item, active: true })).not.toContain("aria-pressed");
  });

  // The palette scrolls the highlighted row into view with `[data-index="…"]`; splitting the row in
  // two moved that attribute onto the wrapper, so pin it where the query can still find it.
  it("carries data-index on the element wrapping both controls", () => {
    expect(row({ index: 4, item: { ...item, starred: true, onToggleStar: () => {} } })).toMatch(
      /^<div data-index="4"/,
    );
  });

  // The row's padding belongs to the selection button: a gutter outside it is a strip of the row
  // that tints on hover and does nothing when clicked.
  it("leaves no unclickable gutter between the selection button and the row edge", () => {
    const html = row({ item, active: true });
    expect(html).not.toMatch(/<div[^>]*class="[^"]*\bpr-2\.5\b/);
    expect(html).toMatch(/<button[^>]*class="[^"]*\bpx-2\.5\b/);
  });
});

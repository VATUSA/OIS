// @vitest-environment jsdom
//
// The palette's keyboard wiring, driven through the DOM. `renderToStaticMarkup` cannot render it —
// `Modal` portals, and the server renderer rejects that — so the parts that only exist once state is
// live (the highlight following its row across a `groups` rebuild, ⌘⇧F, the star swallowing its
// click) are only reachable from here. VATUSA/OIS#312 shipped a highlight bug straight through a
// green suite because the helpers were unit-tested but nothing asserted the component used them.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {CommandPalette} from "./command-palette";

beforeAll(() => {
  // jsdom has no layout, so the palette's scroll-into-view needs a stub.
  Element.prototype.scrollIntoView = () => {};
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

const mounted: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  // The palette portals to `document.body`, so an un-torn-down mount is still in the document and the
  // next test's `document.querySelector("input")` types into it instead.
  for (const { root, host } of mounted.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

const PAGES = [
  { id: "page:/ops/advisories", label: "Advisories" },
  { id: "page:/ops/tmu", label: "TMU" },
  { id: "page:/ops/airport", label: "Airport" },
];

/**
 * The palette wired the way `command-search.tsx` wires it: a pinned Favorites group rebuilt from the
 * favorites list, and every source row carrying the same `entity` as its pinned twin.
 */
function mountPalette(placeholder = "Search…") {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const opened: string[] = [];
  let favorites: string[] = [];

  function Harness() {
    const [, force] = React.useState(0);
    const toggle = (id: string) => {
      favorites = favorites.includes(id) ? favorites.filter((f) => f !== id) : [id, ...favorites];
      force((n) => n + 1);
    };
    const groups = [
      {
        label: "Favorites",
        items: favorites.map((f) => ({
          id: `favorite:${f}`,
          entity: f,
          label: PAGES.find((p) => p.id === f)!.label,
          onSelect: () => opened.push(`favorite:${f}`),
          starred: true,
          onToggleStar: () => toggle(f),
        })),
      },
      {
        label: "Pages",
        items: PAGES.map((p) => ({
          id: p.id,
          entity: p.id,
          label: p.label,
          onSelect: () => opened.push(p.id),
          starred: favorites.includes(p.id),
          onToggleStar: () => toggle(p.id),
        })),
      },
    ];
    return (
      <CommandPalette
        open
        onClose={() => {}}
        query=""
        onQueryChange={() => {}}
        groups={groups}
        placeholder={placeholder}
        empty="No results."
      />
    );
  }

  const root = createRoot(host);
  mounted.push({ root, host });
  act(() => root.render(<Harness />));

  // The palette portals out of `host`, so find this mount's own dialog by the placeholder it renders.
  const input = () =>
    Array.from(document.querySelectorAll<HTMLInputElement>("input")).find((i) => i.placeholder === placeholder)!;
  const panel = () => input().closest("[role]") ?? document.body;
  const key = (init: KeyboardEventInit) =>
    act(() => {
      input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
    });
  const rows = () => Array.from(panel().querySelectorAll<HTMLElement>("button[data-index]"));
  const highlighted = () => rows().find((b) => b.className.includes("bg-panel-2"))?.textContent ?? null;
  const stars = () => Array.from(panel().querySelectorAll<HTMLElement>('[aria-label$="favorites"]'));
  return { key, rows, stars, highlighted, favorites: () => favorites, opened };
}

describe("CommandPalette — the highlight across a groups rebuild", () => {
  it("stays on the arrowed-to row when starring it prepends a favorite above", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    expect(p.highlighted()).toContain("TMU");

    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favorites()).toEqual(["page:/ops/tmu"]);
    expect(p.highlighted()).toContain("TMU");
  });

  it("opens the row the user highlighted, not the one that took its index", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true });
    p.key({ key: "Enter" });
    expect(p.opened).toEqual(["page:/ops/tmu"]);
  });

  it("undoes the star it just added rather than starring a neighbour", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true });
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favorites()).toEqual([]);
  });

  it("follows the entity when the pinned row it is on is un-starred away", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true }); // star Airport
    p.key({ key: "ArrowUp" });
    p.key({ key: "f", metaKey: true, shiftKey: true }); // star TMU
    expect(new Set(p.favorites())).toEqual(new Set(["page:/ops/tmu", "page:/ops/airport"]));

    // Arrow up into the pinned group and land on Airport's favorite row.
    p.key({ key: "ArrowUp" });
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("Airport");

    p.key({ key: "f", metaKey: true, shiftKey: true }); // that row is now gone
    expect(p.favorites()).toEqual(["page:/ops/tmu"]);
    expect(p.highlighted()).toContain("Airport"); // its Pages row, not whoever inherited index 1

    p.key({ key: "f", metaKey: true, shiftKey: true }); // the undo reflex restores Airport
    expect(new Set(p.favorites())).toEqual(new Set(["page:/ops/tmu", "page:/ops/airport"]));
  });

  it("holds its position when a row disappears with no user input", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    mounted.push({ root, host });
    const render = (items: string[]) =>
      act(() =>
        root.render(
          <CommandPalette
            open
            onClose={() => {}}
            query=""
            onQueryChange={() => {}}
            groups={[{ label: "Flights", items: items.map((c) => ({ id: c, label: c, onSelect: () => {} })) }]}
            placeholder="Search…"
            empty="No results."
          />,
        ),
      );
    render(["UAL1", "DAL2", "AAL3", "SWA4"]);
    act(() => {
      document.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
      document.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    const highlighted = () =>
      Array.from(document.querySelectorAll<HTMLElement>("button[data-index]")).find((b) =>
        b.className.includes("bg-panel-2"),
      )?.textContent;
    expect(highlighted()).toBe("AAL3");

    // AAL3 lands and leaves the feed on the next 15s refetch. The highlight must not jump to the top.
    render(["UAL1", "DAL2", "SWA4"]);
    expect(highlighted()).toBe("SWA4");
  });
});

describe("CommandPalette — ⌘⇧F", () => {
  it("toggles the highlighted row on ⌘⇧F and Ctrl+⇧F", () => {
    const meta = mountPalette();
    meta.key({ key: "f", metaKey: true, shiftKey: true });
    expect(meta.favorites()).toEqual(["page:/ops/advisories"]);

    const ctrl = mountPalette("Second palette");
    ctrl.key({ key: "F", ctrlKey: true, shiftKey: true });
    expect(ctrl.favorites()).toEqual(["page:/ops/advisories"]);
  });

  it("leaves browser find (⌘F), find-next (⌘⇧G) and a bare ⇧F alone", () => {
    const p = mountPalette();
    p.key({ key: "f", metaKey: true });
    p.key({ key: "g", metaKey: true, shiftKey: true });
    p.key({ key: "f", shiftKey: true });
    expect(p.favorites()).toEqual([]);
  });

  it("is inert on a row that cannot be favorited", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    mounted.push({ root, host });
    act(() =>
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          query=""
          onQueryChange={() => {}}
          groups={[{ label: "Pages", items: [{ id: "a", label: "A", onSelect: () => {} }] }]}
          placeholder="Search…"
          empty="No results."
        />,
      ),
    );
    expect(() =>
      act(() => {
        document
          .querySelector("input")!
          .dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "f", metaKey: true, shiftKey: true }));
      }),
    ).not.toThrow();
  });
});

describe("CommandPalette — the row star", () => {
  it("toggles the favorite without opening the row", () => {
    const p = mountPalette();
    act(() => p.stars()[0].dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(p.favorites()).toEqual(["page:/ops/advisories"]);
    expect(p.opened).toEqual([]); // the star sits inside the row button — it must swallow the click
  });

  it("keeps a starred row's star, and its pressed state, once the highlight moves on", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" }); // TMU
    p.key({ key: "f", metaKey: true, shiftKey: true });
    p.key({ key: "ArrowDown" }); // Airport — TMU is starred but no longer highlighted
    expect(p.highlighted()).toContain("Airport");

    const tmiStars = p.stars().filter((s) => s.closest("button")?.textContent?.includes("TMU"));
    expect(tmiStars.length).toBeGreaterThan(0);
    for (const s of tmiStars) {
      expect(s.getAttribute("aria-label")).toBe("Remove from favorites");
      expect(s.getAttribute("aria-pressed")).toBe("true");
    }
    // An unstarred, unhighlighted row shows none.
    expect(p.stars().some((s) => s.closest("button")?.textContent?.includes("Advisories"))).toBe(false);
  });
});

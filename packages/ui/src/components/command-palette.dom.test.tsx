// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {CommandPalette} from "./command-palette";

// `renderToStaticMarkup` can assert a row's markup but not the palette's wiring, and the wiring is
// where every #312 bug lived — a raw-index revert used to pass the whole static suite. These drive
// the real component through real keydowns (VATUSA/OIS#312).
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  // jsdom has no layout.
  Element.prototype.scrollIntoView = () => {};
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  // The palette portals to document.body — tear every mount down or the next test keys into this one.
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

/** Mounts the palette with the same group-rebuild behaviour as command-search.tsx. */
function mountPalette() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const opened: string[] = [];
  let favorites: string[] = [];
  // Rows dropped with no user input — the 15s traffic refetch losing a flight (VATUSA/OIS#339).
  let hidden: string[] = [];
  let rerender = () => {};

  function Harness() {
    const [, force] = React.useState(0);
    rerender = () => force((n) => n + 1);
    const toggle = (id: string) => {
      favorites = favorites.includes(id) ? favorites.filter((f) => f !== id) : [id, ...favorites];
      force((n) => n + 1);
    };
    const pages = [
      { id: "page:/ops/advisories", label: "Advisories" },
      { id: "page:/ops/tmu", label: "TMU" },
      { id: "page:/ops/airport", label: "Airport" },
      { id: "page:/admin/planning/events", label: "Events" },
    ].filter((p) => !hidden.includes(p.id));
    const groups = [
      {
        label: "Favorites",
        items: favorites.map((f) => ({
          id: `favorite:${f}`,
          entityId: f,
          label: pages.find((p) => p.id === f)!.label,
          onSelect: () => opened.push(`fav:${f}`),
          starred: true,
          onToggleStar: () => toggle(f),
        })),
      },
      {
        label: "Pages",
        items: pages.map((p) => ({
          id: p.id,
          entityId: p.id,
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
        placeholder="search"
        empty="none"
      />
    );
  }

  const root = createRoot(host);
  roots.push({ root, host });
  act(() => root.render(<Harness />));
  const input = () => document.querySelector("input")!;
  const key = (init: KeyboardEventInit) =>
    act(() => void input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init })));
  // `data-index` sits on the row container, which is a <div> once the star is its own button — match
  // the attribute, not the element, so this holds either way.
  const rows = () => Array.from(document.querySelectorAll("[data-index]"));
  const highlighted = () => rows().find((r) => r.className.includes("bg-panel-2"))?.textContent;
  const starOf = (label: string) =>
    rows()
      .find((r) => r.textContent?.includes(label))
      ?.querySelector<HTMLButtonElement>("button[aria-pressed]");
  // Several ids land in one render, as one refetch does.
  const vanish = (...ids: string[]) => act(() => void ((hidden = [...hidden, ...ids]), rerender()));
  const restore = (...ids: string[]) => act(() => void ((hidden = hidden.filter((h) => !ids.includes(h))), rerender()));
  return { key, rows, highlighted, starOf, vanish, restore, favs: () => favorites, opened };
}

describe("⌘⇧F must not move the highlight (VATUSA/OIS#312)", () => {
  it("keeps the highlight on the arrowed-to row after starring", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    expect(p.highlighted()).toContain("TMU");
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual(["page:/ops/tmu"]);
    expect(p.highlighted()).toContain("TMU");
  });

  it("re-stars the same row when the undo press follows", () => {
    const p = mountPalette();
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual(["page:/ops/advisories"]);
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual([]);
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual(["page:/ops/advisories"]);
  });

  it("with two favorites, undoing the second does not unstar the first", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true });
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("TMU");
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(new Set(p.favs())).toEqual(new Set(["page:/ops/airport", "page:/ops/tmu"]));
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual(["page:/ops/airport"]);
    p.key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual(["page:/ops/tmu", "page:/ops/airport"]);
  });

  it("un-starring a row arrowed to INSIDE the Favorites group holds the highlight", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true }); // Airport
    p.key({ key: "ArrowUp" });
    p.key({ key: "f", metaKey: true, shiftKey: true }); // TMU
    expect(p.favs()).toEqual(["page:/ops/tmu", "page:/ops/airport"]);
    p.key({ key: "ArrowUp" });
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("Airport");
    p.key({ key: "f", metaKey: true, shiftKey: true }); // remove Airport
    expect(p.favs()).toEqual(["page:/ops/tmu"]);
    // The undo press must put *Airport* back and leave TMU alone. Removing the pinned row moved the
    // highlight to the Pages row for the same entity, which is what makes that reachable; toggling
    // prepends, so the restored favorite is first.
    p.key({ key: "f", metaKey: true, shiftKey: true }); // undo
    expect(p.favs()).toEqual(["page:/ops/airport", "page:/ops/tmu"]);
  });

  it("leaves browser find (⌘F) and find-next (⌘⇧G) alone", () => {
    const p = mountPalette();
    p.key({ key: "f", metaKey: true });
    p.key({ key: "g", metaKey: true, shiftKey: true });
    expect(p.favs()).toEqual([]);
  });

  it("arrows from the twin after un-starring a pinned row, not from the top", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    p.key({ key: "f", metaKey: true, shiftKey: true }); // star Airport
    p.key({ key: "ArrowUp" });
    p.key({ key: "ArrowUp" });
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("Airport"); // the pinned row
    p.key({ key: "f", metaKey: true, shiftKey: true }); // un-star it: highlight moves to the Pages twin
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("TMU");
  });

  // M14: the star is its own button beside the row, so activating it cannot select the row.
  it("clicking the star toggles the favorite without opening the row", () => {
    const p = mountPalette();
    const star = p.starOf("Advisories");
    expect(star).toBeTruthy();
    act(() => star!.click());
    expect(p.favs()).toEqual(["page:/ops/advisories"]);
    expect(p.opened).toEqual([]);
  });
});

describe("a row that vanishes with no user input (VATUSA/OIS#339)", () => {
  it("arrows from the held position, not from the top", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    expect(p.highlighted()).toContain("Airport");
    p.vanish("page:/ops/airport");
    expect(p.highlighted()).toContain("Events"); // held at index 2
    p.key({ key: "ArrowUp" });
    expect(p.highlighted()).toContain("TMU");
  });

  it("does not take the highlight back when it returns", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    p.vanish("page:/ops/airport");
    expect(p.highlighted()).toContain("Events");
    p.restore("page:/ops/airport");
    // The user has been reading Events since the refetch; an Enter now must not open Airport.
    expect(p.highlighted()).toContain("Events");
  });
});

describe("a list that renders empty for one tick (VATUSA/OIS#339 review)", () => {
  it("still holds the position the highlight had before it", () => {
    const p = mountPalette();
    p.key({ key: "ArrowDown" });
    p.key({ key: "ArrowDown" });
    expect(p.highlighted()).toContain("Airport");
    p.vanish("page:/ops/advisories", "page:/ops/tmu", "page:/ops/airport", "page:/admin/planning/events");
    expect(p.rows()).toHaveLength(0);
    // Everything but Airport comes back: held at index 2, which is now Events — not the top row.
    p.restore("page:/ops/advisories", "page:/ops/tmu", "page:/admin/planning/events");
    expect(p.highlighted()).toContain("Events");
  });
});

/**
 * One starred row and one plain row, no `scopes` — so nothing holds Tab in the search field and the
 * row's own tab behaviour is what counts. Restored from 59976a3: these were lost when #337 and #338
 * merged, which left the row's two controls without a test that could fail (VATUSA/OIS#340).
 */
function mountRow() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const selected: string[] = [];
  const toggled: string[] = [];

  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <CommandPalette
        open
        onClose={() => {}}
        query=""
        onQueryChange={() => {}}
        groups={[
          {
            label: "Pages",
            items: [
              { id: "tmu", label: "TMU", onSelect: () => selected.push("tmu"), starred: true, onToggleStar: () => toggled.push("tmu") },
              { id: "plain", label: "Advisories", onSelect: () => selected.push("plain") },
            ],
          },
        ]}
        placeholder="Search…"
        empty="No results."
      />,
    ),
  );

  const input = () => document.querySelector<HTMLInputElement>("input")!;
  const star = () => document.querySelector<HTMLButtonElement>('button[aria-label$="favorites"]')!;
  const rowAt = (i: number) => document.querySelector<HTMLElement>(`[data-index="${i}"]`)!;
  const rowButton = () => rowAt(0).querySelector<HTMLButtonElement>(":scope > button")!;
  const highlighted = () =>
    Array.from(document.querySelectorAll<HTMLElement>("[data-index]")).find((r) =>
      r.className.includes("bg-panel-2"),
    )?.textContent;
  const click = (el: HTMLElement) =>
    act(() => {
      el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  return { input, star, rowButton, rowAt, highlighted, click, selected, toggled };
}

describe("the row's two controls (VATUSA/OIS#336, #340)", () => {
  it("toggles the favorite and does NOT select the row when the star is activated", () => {
    const p = mountRow();
    p.click(p.star());
    expect(p.toggled).toEqual(["tmu"]);
    expect(p.selected).toEqual([]);
  });

  it("selects the row, and does not toggle, when the row button is activated", () => {
    const p = mountRow();
    p.click(p.rowButton());
    expect(p.selected).toEqual(["tmu"]);
    expect(p.toggled).toEqual([]);
  });

  // The gutter beside and around the star is the row's own surface. It highlights on hover, so it
  // has to select on click — not sit there dead, and not toggle a favorite by accident.
  it("selects the row, and does not toggle, when the gutter around the star is clicked", () => {
    const p = mountRow();
    p.click(p.rowAt(0));
    expect(p.selected).toEqual(["tmu"]);
    expect(p.toggled).toEqual([]);
  });

  // ⌘⇧F is the star's keyboard path. A tabbable star could hold focus on a starred row that isn't
  // highlighted, and un-starring unmounts it — focus falls to <body>, where no shortcut listens.
  it("keeps the star out of the tab order", () => {
    const p = mountRow();
    expect(p.star().tabIndex).toBe(-1);
  });

  it("cancels the star's mousedown, which is what holds focus in the search field", () => {
    const p = mountRow();
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    act(() => void p.star().dispatchEvent(down));
    expect(down.defaultPrevented).toBe(true);
  });

  it("still toggles the highlighted row on ⌘⇧F, and leaves plain ⌘F alone", () => {
    const p = mountRow();
    const key = (init: KeyboardEventInit) =>
      act(() => void p.input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init })));
    key({ key: "f", metaKey: true });
    expect(p.toggled).toEqual([]);
    key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.toggled).toEqual(["tmu"]);
    expect(p.selected).toEqual([]);
  });

  it("highlights the row the pointer moves over, from anywhere in it", () => {
    const p = mountRow();
    expect(p.highlighted()).toContain("TMU");
    act(() => void p.rowAt(1).dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(p.highlighted()).toContain("Advisories");
    act(() => void p.star().dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(p.highlighted()).toContain("TMU");
  });

  it("scrolls the highlighted row into view by its data-index", () => {
    const p = mountRow();
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.getAttribute("data-index") ?? "none");
    };
    try {
      act(() => void p.input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    } finally {
      Element.prototype.scrollIntoView = () => {};
    }
    expect(scrolled).toContain("1");
  });
});

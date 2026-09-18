// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeEach, describe, expect, it, vi} from "vitest";

import {CommandPalette, type CommandGroup, type CommandItem} from "./command-palette";

// The palette's keyboard and star behaviour only exists in a DOM, and `renderToStaticMarkup` can't
// reach it — `Modal` portals, which the server renderer rejects.
(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: ReturnType<typeof createRoot>;
const render = (ui: React.ReactElement) => act(() => root.render(ui));

beforeEach(() => {
  document.body.innerHTML = "";
  // jsdom has no layout engine; the palette scrolls the highlighted row into view.
  Element.prototype.scrollIntoView = () => {};
  const node = document.createElement("div");
  document.body.appendChild(node);
  root = createRoot(node);
});

/** The palette portals to `document.body`, so queries run against the document, not the mount node. */
const press = (init: KeyboardEventInit) =>
  act(() => {
    document.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
  });

const highlighted = () => document.querySelector<HTMLElement>("[data-index].bg-panel-2")?.textContent ?? null;

const item = (over: Partial<CommandItem> = {}): CommandItem => ({
  id: "a",
  label: "Row A",
  onSelect: vi.fn(),
  starred: false,
  onToggleStar: vi.fn(),
  ...over,
});

const show = (groups: CommandGroup[]) =>
  render(<CommandPalette open onClose={() => {}} query="" onQueryChange={() => {}} groups={groups} />);

const one = (items: CommandItem[]) => show([{ label: "Favorites", items }]);

describe("⌘⇧F toggles the highlighted row, and only that combo", () => {
  it("fires on ⌘⇧F and on Ctrl+⇧F", () => {
    const onToggleStar = vi.fn();
    one([item({ onToggleStar })]);
    press({ key: "f", metaKey: true, shiftKey: true });
    press({ key: "F", ctrlKey: true, shiftKey: true });
    expect(onToggleStar).toHaveBeenCalledTimes(2);
  });

  // The whole point of the Shift: ⌘F is browser find and ⌘G is find-next (VATUSA/OIS#312).
  it("does not fire on ⌘F, ⌘G or a bare ⇧F", () => {
    const onToggleStar = vi.fn();
    one([item({ onToggleStar })]);
    press({ key: "f", metaKey: true });
    press({ key: "g", metaKey: true });
    press({ key: "f", shiftKey: true });
    expect(onToggleStar).not.toHaveBeenCalled();
  });

  it("toggles the arrowed-to row, not always the first", () => {
    const first = vi.fn();
    const second = vi.fn();
    one([item({ id: "a", onToggleStar: first }), item({ id: "b", onToggleStar: second })]);
    press({ key: "ArrowDown" });
    press({ key: "f", metaKey: true, shiftKey: true });
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });

  it("is inert on a row with no star", () => {
    one([item({ onToggleStar: undefined })]);
    expect(() => press({ key: "f", metaKey: true, shiftKey: true })).not.toThrow();
  });
});

describe("the highlight follows the row, not its index", () => {
  /** A pinned Favorites group over a fixed Pages group — the shape `command-search` builds. */
  const build = (favs: string[], toggle: (label: string) => void): CommandGroup[] => [
    {
      label: "Favorites",
      items: favs.map((l) => ({
        id: `fav:${l}`,
        label: `★ ${l}`,
        onSelect: () => {},
        starred: true,
        onToggleStar: () => toggle(l),
      })),
    },
    {
      label: "Pages",
      items: ["Advisories", "TMU", "Runway"].map((l) => ({
        id: `page:${l}`,
        label: l,
        onSelect: () => {},
        starred: favs.includes(l),
        onToggleStar: () => toggle(l),
      })),
    },
  ];

  /** Stars via the palette, re-rendering with the new groups exactly as the caller would. */
  const starrable = () => {
    let favs: string[] = [];
    const toggle = (l: string) => {
      favs = favs.includes(l) ? favs.filter((f) => f !== l) : [l, ...favs];
      show(build(favs, toggle));
    };
    show(build(favs, toggle));
    return {
      get favs() {
        return favs;
      },
    };
  };

  // Starring prepends a row to the pinned group, shifting every index after it. Holding the
  // highlight as an index silently moved it to the row above (VATUSA/OIS#312).
  it("keeps the highlight on the row the user just starred", () => {
    starrable();
    press({ key: "ArrowDown" });
    expect(highlighted()).toBe("TMU");
    press({ key: "f", metaKey: true, shiftKey: true });
    expect(highlighted()).toBe("TMU");
  });

  it("lets a second ⌘⇧F un-star the same row", () => {
    const s = starrable();
    press({ key: "ArrowDown" });
    press({ key: "f", metaKey: true, shiftKey: true });
    expect(s.favs).toEqual(["TMU"]);
    press({ key: "f", metaKey: true, shiftKey: true });
    expect(s.favs).toEqual([]);
  });

  it("opens the starred row on Enter, not its neighbour", () => {
    let opened: string | null = null;
    let favs: string[] = [];
    const groups = (): CommandGroup[] => [
      {
        label: "Favorites",
        items: favs.map((l) => ({ id: `fav:${l}`, label: `★ ${l}`, onSelect: () => (opened = `★ ${l}`) })),
      },
      {
        label: "Pages",
        items: ["Advisories", "TMU", "Runway"].map((l) => ({
          id: `page:${l}`,
          label: l,
          onSelect: () => (opened = l),
          starred: favs.includes(l),
          onToggleStar: () => {
            favs = [l, ...favs];
            show(groups());
          },
        })),
      },
    ];
    show(groups());
    press({ key: "ArrowDown" });
    press({ key: "f", metaKey: true, shiftKey: true });
    press({ key: "Enter" });
    expect(opened).toBe("TMU");
  });

  it("falls back to the first row when the highlighted one disappears", () => {
    one([item({ id: "a", label: "Row A" }), item({ id: "b", label: "Row B" })]);
    press({ key: "ArrowDown" });
    expect(highlighted()).toBe("Row B");
    one([item({ id: "a", label: "Row A" })]);
    expect(highlighted()).toBe("Row A");
  });

  it("selects the first row on Enter after the highlighted one disappears", () => {
    const onSelect = vi.fn();
    one([item({ id: "a" }), item({ id: "b", label: "Row B" })]);
    press({ key: "ArrowDown" });
    one([item({ id: "a", label: "Row A", onSelect })]);
    press({ key: "Enter" });
    expect(onSelect).toHaveBeenCalledTimes(1);
  });
});

describe("the row star", () => {
  it("toggles without opening the row", () => {
    const onSelect = vi.fn();
    const onToggleStar = vi.fn();
    one([item({ starred: true, onSelect, onToggleStar })]);
    const star = document.querySelector<HTMLElement>('[aria-label="Remove from favorites"]')!;
    act(() => star.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onToggleStar).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
  });

  // A favorite has to be visible to be un-favoritable without hunting for it with the mouse.
  it("stays visible on a starred row that isn't highlighted", () => {
    one([item({ id: "a" }), item({ id: "b", starred: true })]);
    const rows = document.querySelectorAll("[data-index]");
    expect(rows[1].querySelector('[aria-label="Remove from favorites"]')).not.toBeNull();
  });

  it("is absent from an unstarred row that isn't highlighted", () => {
    one([item({ id: "a" }), item({ id: "b" })]);
    expect(document.querySelectorAll("[data-index]")[1].querySelector("[aria-pressed]")).toBeNull();
  });

  it("reports its state through aria-pressed and aria-label", () => {
    one([item({ starred: true })]);
    expect(document.querySelector("[aria-pressed]")!.getAttribute("aria-pressed")).toBe("true");
    one([item({ starred: false })]);
    expect(document.querySelector("[aria-pressed]")!.getAttribute("aria-label")).toBe("Add to favorites");
  });

  it("is not rendered at all when the row has no onToggleStar (signed out)", () => {
    one([item({ onToggleStar: undefined, starred: false })]);
    expect(document.querySelector("[aria-pressed]")).toBeNull();
  });
});

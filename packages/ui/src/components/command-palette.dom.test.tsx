// @vitest-environment jsdom
//
// What activating each of the row's two controls actually does. `renderToStaticMarkup` can pin the
// markup apart, but the two behaviours this issue turns on — a star click must not select the row,
// and must not steal focus from the search field — only exist once events are live, and `Modal`
// portals so the palette can't be server-rendered at all (VATUSA/OIS#336).
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
  // The palette portals to `document.body`, so an un-torn-down mount is still in the document and
  // the next test's queries would find it instead.
  for (const { root, host } of mounted.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

function mountPalette() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const selected: string[] = [];
  const toggled: string[] = [];

  const root = createRoot(host);
  mounted.push({ root, host });
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
  const rowButton = () => document.querySelector<HTMLButtonElement>('[data-index="0"] > button')!;
  const rowAt = (i: number) => document.querySelector<HTMLElement>(`[data-index="${i}"]`)!;
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

describe("the row's two controls", () => {
  it("toggles the favorite and does NOT select the row when the star is activated", () => {
    const p = mountPalette();
    p.click(p.star());
    expect(p.toggled).toEqual(["tmu"]);
    expect(p.selected).toEqual([]);
  });

  it("selects the row, and does not toggle, when the row button is activated", () => {
    const p = mountPalette();
    p.click(p.rowButton());
    expect(p.selected).toEqual(["tmu"]);
    expect(p.toggled).toEqual([]);
  });

  it("keeps focus in the search field when the star is clicked", () => {
    const p = mountPalette();
    p.input().focus();
    expect(document.activeElement).toBe(p.input());
    p.click(p.star());
    // The star's mousedown is prevented, so the browser never moves focus to it — the arrows, Enter,
    // tab-scope and ⌘⇧F all keep working straight after a click.
    expect(document.activeElement).toBe(p.input());
  });

  it("cancels the star's mousedown, which is what holds focus where it is", () => {
    const p = mountPalette();
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    act(() => void p.star().dispatchEvent(down));
    expect(down.defaultPrevented).toBe(true);
  });

  it("still toggles the highlighted row on ⌘⇧F, and leaves plain ⌘F alone", () => {
    const p = mountPalette();
    const key = (init: KeyboardEventInit) =>
      act(() => void p.input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init })));
    key({ key: "f", metaKey: true });
    expect(p.toggled).toEqual([]);
    key({ key: "f", metaKey: true, shiftKey: true });
    expect(p.toggled).toEqual(["tmu"]);
    expect(p.selected).toEqual([]);
  });

  // The wrapper took this over from the row button when the two controls were split apart, so the
  // whole row — the star included — still highlights on hover.
  it("highlights the row the pointer moves over, from anywhere in it", () => {
    const p = mountPalette();
    expect(p.highlighted()).toContain("TMU");
    act(() => void p.rowAt(1).dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(p.highlighted()).toContain("Advisories");
    act(() => void p.star().dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(p.highlighted()).toContain("TMU");
  });

  it("scrolls the highlighted row into view by its data-index", () => {
    const p = mountPalette();
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.getAttribute("data-index") ?? "none");
    };
    act(() => void p.input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    Element.prototype.scrollIntoView = () => {};
    expect(scrolled).toContain("1");
  });
});

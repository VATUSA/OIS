// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {Modal} from "@ois/ui";
import {afterEach, describe, expect, it} from "vitest";

import {CHANGELOG, type ChangelogEntry, type Screenshot} from "@/lib/changelog";

import {ChangelogList, openWhatsNew, WhatsNew} from "./whats-new";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

const mounted: {root: Root; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of mounted.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

function mount(node: React.ReactNode, qc = new QueryClient()) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>));
  mounted.push({root, host});
}

const shots = (count: number): Screenshot[] =>
  Array.from({length: count}, (_, i) => ({src: `/shot-${i}.png`, alt: `Shot ${i}`}));
const withShots = (count: number): ChangelogEntry => ({
  id: "e",
  date: "2026-10-01",
  title: "Entry",
  sections: [{heading: "Section", highlights: ["A change"], shots: shots(count)}],
});

const grid = () => document.querySelector<HTMLElement>("[data-testid=shot-grid]")!;
const dialogs = () => [...document.querySelectorAll("[role=dialog]")];
const escape = () =>
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape"}));
  });

describe("the shot grid (#665)", () => {
  it.each([
    [1, "grid-cols-1", null],
    [2, "sm:grid-cols-2", "sm:grid-cols-3"],
    [4, "sm:grid-cols-2", "sm:grid-cols-3"],
    [8, "sm:grid-cols-3", "sm:grid-cols-2"],
  ])("lays out %i shots with %s", (count, expected, absent) => {
    mount(<ChangelogList entries={[withShots(count)]} />);
    expect(grid().className).toContain(expected);
    // One column on a narrow screen, always.
    expect(grid().className).toContain("grid-cols-1");
    if (absent) expect(grid().className).not.toContain(absent);
    expect(grid().querySelectorAll("img")).toHaveLength(count);
  });

  it("renders a single unheaded section as a plain list, like before sections existed", () => {
    mount(
      <ChangelogList entries={[{id: "e", date: "2026-10-01", title: "Entry", sections: [{highlights: ["One", "Two"]}]}]} />,
    );
    expect(document.querySelector("h3")).toBeNull();
    expect([...document.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["One", "Two"]);
    expect(document.querySelector("[data-testid=shot-grid]")).toBeNull();
  });

  it("enlarges a shot in a dialog that Escape closes", () => {
    mount(<ChangelogList entries={[withShots(2)]} />);
    expect(dialogs()).toHaveLength(0);
    act(() => grid().querySelector<HTMLButtonElement>("button")!.click());
    expect(dialogs()).toHaveLength(1);
    expect(dialogs()[0]!.querySelector("img")!.getAttribute("src")).toBe("/shot-0.png");
    escape();
    expect(dialogs()).toHaveLength(0);
  });
});

describe("the panel (#665)", () => {
  function signedInSeen() {
    const qc = new QueryClient({defaultOptions: {queries: {retry: false, staleTime: Infinity}}});
    qc.setQueryData(["me"], {id: "u1", display_name: "Test", cid: 1, permissions: []});
    qc.setQueryData(["preferences", "changelog"], {lastSeenId: CHANGELOG[0]!.id});
    return qc;
  }

  it("reopens with every entry, and closing it does not mark anything seen", () => {
    const qc = signedInSeen();
    mount(<WhatsNew />, qc);
    expect(dialogs()).toHaveLength(0);

    act(() => openWhatsNew());
    const text = document.body.textContent ?? "";
    for (const entry of CHANGELOG) expect(text).toContain(entry.title);

    act(() => document.querySelector<HTMLButtonElement>("[aria-label=Close]")!.click());
    expect(dialogs()).toHaveLength(0);
    expect(qc.getMutationCache().getAll()).toHaveLength(0);
  });

  it("keeps the enlarged view's Escape from closing the panel", () => {
    // The panel's own entries carry no shots yet, so nest a grid in a real Modal to test the pair.
    mount(<NestedPanel />);
    act(() => document.querySelector<HTMLButtonElement>("[data-testid=shot-grid] button")!.click());
    expect(dialogs()).toHaveLength(2);
    escape();
    expect(dialogs()).toHaveLength(1);
    escape();
    expect(dialogs()).toHaveLength(0);
  });
});

function NestedPanel() {
  const [open, setOpen] = React.useState(true);
  return open ? <PanelModal onClose={() => setOpen(false)} /> : null;
}

function PanelModal({onClose}: {onClose: () => void}) {
  return (
    <Modal open onClose={onClose} title="What's new" size="xl">
      <ChangelogList entries={[withShots(2)]} />
    </Modal>
  );
}

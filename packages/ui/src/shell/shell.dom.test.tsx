// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {Sparkles} from "lucide-react";

import {Shell, ShellContent, SidebarItem} from "./shell";

/**
 * The shell's own chrome and its top-bar seam (#402).
 *
 * Nothing asserted either before, and the shell wraps every signed-in page. The gutter, the outer
 * rounded corners and the frame shadow were removed so the app meets the window edges squarely, and
 * on the desktop build a gutter here would read as a second chrome bar under the hidden title bar.
 * The *inner* content panel keeps its rounding and margins — the part that is easy to remove by
 * accident while "flattening the shell", and now the only thing holding content off the window edge.
 *
 * The top bar is what moves a frameless window, so it takes the Tauri drag region as an opaque prop:
 * this package stays free of platform assumptions, which also means nothing here would notice if the
 * props stopped reaching the element.
 */
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

function render(node: React.ReactNode): HTMLElement {
  const host = document.createElement("div");
  root = createRoot(host);
  act(() => root!.render(node));
  return host;
}

/** The shell's outermost element, and the frame inside it. */
function shellParts(host: HTMLElement) {
  const outer = host.firstElementChild as HTMLElement;
  return {outer, frame: outer.firstElementChild as HTMLElement};
}

describe("Shell chrome", () => {
  it("meets the viewport edges: no gutter, no outer radius, no shadow", () => {
    const {outer, frame} = shellParts(render(<Shell>page</Shell>));

    // A gutter of any breakpoint would put --ground back around the app.
    for (const gutter of ["p-2", "sm:p-3", "p-3"]) {
      expect(outer.className).not.toContain(gutter);
    }
    expect(frame.className).not.toContain("rounded");
    expect(frame.className).not.toContain("shadow");
    // Still full height, and still the panel surface the sidebar and main area sit on.
    expect(outer.className).toContain("h-dvh");
    expect(frame.className).toContain("bg-panel");
  });

  it("keeps the inner content panel inset and rounded", () => {
    const host = render(<Shell>page</Shell>);
    const panel = host.querySelector(".rounded-lg") as HTMLElement | null;

    expect(panel, "the inner content panel should survive flattening the frame").not.toBeNull();
    expect(panel!.className).toContain("border");
    // The inset is load-bearing now, not decoration: with the frame's gutter gone this margin is the
    // only thing holding the content panel off the window edge, so assert it rather than just the
    // rounding it is named after.
    expect(panel!.className).toContain("mx-2");
    expect(panel!.className).toContain("mb-2");
    expect(panel!.textContent).toBe("page");
  });

  it("does not paint a ground behind the frame, which would never be visible", () => {
    // The frame is the outer element's only child and stretches over the whole h-dvh box, so a
    // `bg-ground` out here is dead weight that reads as though a gutter were coming back.
    const {outer} = shellParts(render(<Shell>page</Shell>));

    expect(outer.className).not.toContain("bg-ground");
  });

  it("gets its side gutters from the content padding, not from the frame", () => {
    // AC2: with the frame's gutter gone, 16px at phone width has to come from here or pages sit
    // flush against the window edge.
    const host = render(
      <Shell>
        <ShellContent>body</ShellContent>
      </Shell>,
    );

    const padded = host.querySelector(".px-4") as HTMLElement | null;
    expect(padded, "ShellContent should still carry its own horizontal padding").not.toBeNull();
    expect(padded!.className).toContain("sm:px-6");
  });
});

describe("Shell top bar", () => {
  it("passes the drag region through to the top bar", () => {
    const host = render(
      <Shell topBarProps={{"data-tauri-drag-region": true}} breadcrumbs={<span>crumbs</span>}>
        page
      </Shell>,
    );

    const dragRegion = host.querySelector("[data-tauri-drag-region]");
    expect(dragRegion, "the top bar should carry the drag region").not.toBeNull();
    // It has to be the bar itself: Tauri's drag matching is self-only for a bare/"true" attribute, so
    // on a child only that child would move the window.
    expect(dragRegion!.textContent).toContain("crumbs");
  });

  it("adds nothing to the top bar on the web build, which passes no props", () => {
    const host = render(<Shell>page</Shell>);

    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });

  it("cannot have its layout classes replaced by a caller's props", () => {
    // The seam is opaque on purpose, but `className` is not a prop to hand over: spread after it, a
    // caller's className silently drops the row's height and padding. The type forbids it — this
    // pins that the spread order does too, for a caller that gets past the type.
    const host = render(
      <Shell topBarProps={{id: "top-bar", className: "p-0"} as never}>page</Shell>,
    );

    const bar = host.querySelector("#top-bar") as HTMLElement;
    expect(bar.className).toContain("h-11");
    expect(bar.className).toContain("px-3");
    expect(bar.className).not.toBe("p-0");
  });
});

describe("SidebarItem (#680)", () => {
  // A <button> centres its text by default, unlike the <a> every other item is; the item must not
  // depend on which element it is handed.
  it("left-aligns and fills the row whatever element it renders", () => {
    const host = render(
      <SidebarItem asChild icon={Sparkles} label="What's new">
        <button type="button" />
      </SidebarItem>,
    );
    const button = host.querySelector("button")!;
    expect(button.className).toContain("text-left");
    expect(button.className).toContain("w-full");
  });

  it("passes iconClassName to the icon", () => {
    const host = render(<SidebarItem href="#" icon={Sparkles} label="x" iconClassName="custom-icon" />);
    expect(host.querySelector("svg")!.getAttribute("class")).toContain("custom-icon");
  });
});

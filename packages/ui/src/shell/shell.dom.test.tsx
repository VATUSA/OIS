// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {Shell} from "./shell";

/**
 * The shell's top-bar seam (#402).
 *
 * The desktop app has no native title bar, so this row is what moves the window: it carries the Tauri
 * drag region and the double-click-to-maximize handler. `Shell` takes them as opaque props so this
 * package stays free of platform assumptions — which also means nothing here would notice if the
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

describe("Shell top bar", () => {
  it("passes the drag region through to the top bar", () => {
    const host = render(
      <Shell topBarProps={{"data-tauri-drag-region": true}} breadcrumbs={<span>crumbs</span>}>
        page
      </Shell>,
    );

    const dragRegion = host.querySelector("[data-tauri-drag-region]");
    expect(dragRegion, "the top bar should carry the drag region").not.toBeNull();
    // It has to be the bar itself: on a child, only that child would move the window.
    expect(dragRegion!.textContent).toContain("crumbs");
  });

  it("calls the top bar's double-click handler, which is how a frameless window maximizes", () => {
    let doubles = 0;
    const host = render(<Shell topBarProps={{onDoubleClick: () => (doubles += 1)}}>page</Shell>);

    const bar = host.querySelector(".h-11") as HTMLElement;
    act(() => bar.dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));

    expect(doubles).toBe(1);
  });

  it("adds nothing to the top bar on the web build, which passes no props", () => {
    const host = render(<Shell>page</Shell>);

    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });
});

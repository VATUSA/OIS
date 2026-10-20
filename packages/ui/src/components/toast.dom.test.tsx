// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {ToastProvider, useToast} from "./toast";

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
  document.body.innerHTML = "";
});

describe("ToastProvider (#824)", () => {
  // Pushing a toast changes only the provider's own list. If the context value were a new object on
  // every render, every `useToast` consumer would re-render on each toast, and a page whose table
  // columns depend on anything derived from it would remount its cells, so an armed ConfirmButton
  // in a cell came back idle before its second click.
  it("does not re-render other useToast consumers when a toast is pushed", () => {
    const seen: ReturnType<typeof useToast>[] = [];
    function Watcher() {
      seen.push(useToast());
      return null;
    }
    function Pusher() {
      const toast = useToast();
      return (
        <button type="button" onClick={() => toast.warning("Discard this draft?")}>
          Arm
        </button>
      );
    }

    const host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    // Built once: the provider's own re-render then passes the same elements, so only a context
    // change can re-render Watcher.
    const tree = (
      <ToastProvider>
        <Watcher />
        <Pusher />
      </ToastProvider>
    );
    act(() => root!.render(tree));
    expect(seen).toHaveLength(1);

    act(() => host.querySelector("button")!.dispatchEvent(new MouseEvent("click", {bubbles: true})));

    // Positive control: the push went through and the toast is on screen.
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Discard this draft?");
    expect(seen).toHaveLength(1);
  });
});

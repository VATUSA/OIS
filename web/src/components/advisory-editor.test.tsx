// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {AdvisoryEditor} from "./advisory-editor";
import {EMPTY_REROUTE, type Reroute} from "@/lib/advisories";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

/** Render the editor as a controlled component, returning the latest value the caller was handed. */
function mount(initial: Reroute = EMPTY_REROUTE) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  const state = { value: initial };

  function Harness() {
    const [value, setValue] = React.useState(initial);
    state.value = value;
    return <AdvisoryEditor value={value} onChange={setValue} />;
  }
  act(() => root.render(<Harness />));
  return { host, state };
}

const byLabel = (host: HTMLElement, label: string) =>
  host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`);

/** The input inside the `<label>` whose text starts with `text` — `Field` labels by wrapping, which
 *  is correct accessible labelling, so there is no `aria-label` to query for. */
function byFieldLabel(host: HTMLElement, text: string): HTMLInputElement {
  const label = [...host.querySelectorAll("label")].find((l) => l.textContent?.startsWith(text));
  const input = label?.querySelector("input");
  if (!input) throw new Error(`no input under a label starting "${text}"`);
  return input;
}

function type(el: HTMLInputElement, value: string) {
  act(() => {
    // React tracks the last value it set; assigning through the prototype setter is what makes the
    // synthetic change event carry the new value.
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function click(el: Element) {
  act(() => el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

describe("AdvisoryEditor routes table (VATUSA/OIS#460 AC4)", () => {
  it("starts with one single-segment row, because routes is required", () => {
    // `routes` is non-optional on RerouteAdvisory, so an empty table could never be saved — the form
    // must open with a row rather than making the author add one before anything is valid.
    const { state } = mount();
    expect(state.value.routes.kind).toBe("single");
    if (state.value.routes.kind === "single") expect(state.value.routes.rows).toHaveLength(1);
  });

  it("edits a row through fields, with no text alignment involved", () => {
    const { host, state } = mount();
    type(byLabel(host, "Row 1 origin")!, "JFK");
    type(byLabel(host, "Row 1 destination")!, "BOS");
    type(byLabel(host, "Row 1 route")!, "CAMRN ><J79>< BOS");
    const routes = state.value.routes;
    expect(routes.kind).toBe("single");
    if (routes.kind === "single") {
      expect(routes.rows[0]).toEqual({ orig: "JFK", dest: "BOS", route: "CAMRN ><J79>< BOS" });
    }
  });

  it("adds and removes rows without disturbing the others", () => {
    const { host, state } = mount();
    type(byLabel(host, "Row 1 origin")!, "JFK");
    click([...host.querySelectorAll("button")].find((b) => b.textContent?.includes("Add row"))!);
    type(byLabel(host, "Row 2 origin")!, "EWR");

    let routes = state.value.routes;
    if (routes.kind === "single") expect(routes.rows.map((r) => r.orig)).toEqual(["JFK", "EWR"]);

    click(byLabel(host, "Remove row 1")!);
    routes = state.value.routes;
    // The surviving row keeps its own value — a splice bug would leave "JFK" or blank both.
    if (routes.kind === "single") expect(routes.rows.map((r) => r.orig)).toEqual(["EWR"]);
  });

  it("never removes the last row", () => {
    const { host } = mount();
    expect(byLabel(host, "Remove row 1")!.hasAttribute("disabled")).toBe(true);
  });

  it("switches to the segmented shape, which carries no DEST column", () => {
    const { host, state } = mount();
    click([...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Segmented")!);
    expect(state.value.routes.kind).toBe("segmented");

    type(byLabel(host, "origin 1 origin")!, "JFK");
    type(byLabel(host, "destination 1 origin")!, "BOS");
    const routes = state.value.routes;
    if (routes.kind === "segmented") {
      expect(routes.origin[0].orig).toBe("JFK");
      expect(routes.destination[0].orig).toBe("BOS");
      // A segment is {orig, route} — no `dest`, which is the whole distinction from a single row.
      expect(Object.keys(routes.origin[0]).sort()).toEqual(["orig", "route"]);
    }
  });
});

describe("AdvisoryEditor fields", () => {
  it("keeps the valid period as written, not parsed", () => {
    // DDHHMM verbatim: the model's comment says a round-trip through a timestamp would invent a
    // month and year the document never states.
    const { host, state } = mount();
    type(byFieldLabel(host, "Valid from"), "141800");
    expect(state.value.valid.from).toBe("141800");
  });

  it("renders no document preview of its own", () => {
    // The backend derives the document from `structured`; a preview rendered here would be a second
    // implementation of `advisory.rs` and could disagree with what actually posts (#455).
    const { host } = mount();
    expect(host.querySelector('[aria-label="Rendered advisory"]')).toBeNull();
    expect(host.querySelector("pre")).toBeNull();
  });
});

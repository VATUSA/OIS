// @vitest-environment jsdom
//
// VATUSA/OIS#541 AC5. Clicking a stand did nothing at all for a user without
// `flow.surface_data.update` — the handler returned early — so hover was their only route to a
// stand's name, and hover was broken. A read-only user could not identify a stand.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {StandDetailCard} from "./stand-detail";
import type {AirportGate} from "@/lib/airport-surface";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

/** A fully-populated X-Plane stand, matching KDCA E57 in the committed extract. */
const E57: AirportGate = {
  id: "g1",
  icao: "KDCA",
  name: "E57",
  lat: 38.85853226,
  lon: -77.04336496,
  source: "xplane",
  kind: "gate",
  heading: 209.1,
  size_code: "B",
  operation_type: "airline",
  aircraft_classes: ["heavy", "jets"],
  airline_codes: ["aal", "dal"],
  updated_at: "2026-10-01T00:00:00Z",
  editable: false,
};

function render(gate: AirportGate, onClose: () => void = () => {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  act(() =>
    root.render(
      <ToastProvider>
        <StandDetailCard gate={gate} onClose={onClose} />
      </ToastProvider>,
    ),
  );
  return host;
}

describe("StandDetailCard (VATUSA/OIS#541)", () => {
  /** AC5, the whole point: the name has to be there. */
  it("shows the stand's name", () => {
    expect(render(E57).textContent).toContain("E57");
  });

  /** The fields #541 added to the model are the reason the card exists. */
  it("shows the X-Plane detail the import now keeps", () => {
    const text = render(E57).textContent ?? "";

    expect(text).toContain("ICAO B");
    expect(text).toContain("209°");
    expect(text).toContain("airline");
    expect(text).toContain("heavy, jets");
    expect(text).toContain("AAL, DAL");
    expect(text).toContain("xplane");
  });

  /**
   * A hand-entered stand has none of it. Rendering a row per absent field would show a column of
   * dashes, so absent fields are omitted — but the name and source must still be there, or the card
   * is useless for exactly the rows an operator created.
   */
  it("omits absent detail instead of rendering blank rows", () => {
    const manual: AirportGate = {
      ...E57,
      name: "HAND ENTERED",
      source: "manual",
      kind: null,
      heading: null,
      size_code: null,
      operation_type: null,
      aircraft_classes: null,
      airline_codes: null,
    };

    const text = render(manual).textContent ?? "";

    expect(text).toContain("HAND ENTERED");
    expect(text).toContain("manual");
    for (const label of ["Size", "Heading", "Operation", "Aircraft", "Airlines", "Type"]) {
      expect(text, `${label} should be omitted, not empty`).not.toContain(label);
    }
  });

  /** An empty list is not the same as no restriction being recorded; neither should print a row. */
  it("treats an empty list as nothing to show", () => {
    const text = render({...E57, aircraft_classes: [], airline_codes: []}).textContent ?? "";

    expect(text).not.toContain("Aircraft");
    expect(text).not.toContain("Airlines");
    expect(text).toContain("E57");
  });

  /** The card is the only thing on screen for a read-only user, so they must be able to dismiss it. */
  it("can be closed", () => {
    let closed = 0;
    const host = render(E57, () => {
      closed += 1;
    });

    const btn = host.querySelector<HTMLButtonElement>('[aria-label="Close stand details"]');
    expect(btn).not.toBeNull();
    act(() => btn!.click());

    expect(closed).toBe(1);
  });

  /** A stand type reads as a label, not as the database's snake_case. */
  it("renders a tie-down readably", () => {
    expect(render({...E57, kind: "tie_down"}).textContent).toContain("tie down");
  });
});

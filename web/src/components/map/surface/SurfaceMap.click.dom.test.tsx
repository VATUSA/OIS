// @vitest-environment jsdom
//
// VATUSA/OIS#541 AC5, tested through the click handler rather than through the card alone.
//
// The card rendering correctly proves nothing about the bug: the defect was in `handleClick`, which
// returned early for a user without `flow.surface_data.update` so the card was never reached. These
// drive the real handler.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {AirportSurface} from "@/lib/airport-surface";

/** Captures the props MapCanvas is handed, so the real `onClick` can be invoked. */
const canvas = vi.hoisted(() => ({
  onClick: undefined as ((info: unknown, event: unknown) => void) | undefined,
}));

vi.mock("../MapCanvas", () => ({
  MapCanvas: (props: {
    onClick?: (info: unknown, event: unknown) => void;
    children?: React.ReactNode;
  }) => {
    canvas.onClick = props.onClick;
    return props.children ?? null;
  },
}));
vi.mock("../lib/palette", () => ({useMapPalette: () => ({ink: [255, 255, 255], line: [0, 0, 0]})}));

import {SurfaceMap} from "./SurfaceMap";

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
  canvas.onClick = undefined;
});

const E57 = {
  id: "g1",
  icao: "KDCA",
  name: "E57",
  lat: 38.8585,
  lon: -77.0434,
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

const SURFACE = {
  gates: [E57],
  taxiways: [],
  runways: [],
  ramp_areas: [],
} as unknown as AirportSurface;

function mount(editable: boolean) {
  const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <SurfaceMap icao="KDCA" surface={SURFACE} editable={editable} />
        </ToastProvider>
      </QueryClientProvider>,
    ),
  );
  return host;
}

/** A pick on the gate layer, the shape deck hands the handler. */
const gatePick = {layer: {id: "surface-gates"}, object: E57, coordinate: [-77.0434, 38.8585]};

describe("SurfaceMap stand click (VATUSA/OIS#541)", () => {
  /**
   * The regression, stated as AC5 words it. Before the fix this click was swallowed by
   * `if (!editable) return;` and nothing appeared.
   */
  it("shows a stand's name on click without flow.surface_data.update", () => {
    const host = mount(false);
    expect(host.textContent).not.toContain("E57");

    act(() => canvas.onClick!(gatePick, {}));

    expect(host.textContent).toContain("E57");
  });

  /** And the detail that #541 started persisting rides along. */
  it("shows the imported detail for a read-only user", () => {
    const host = mount(false);

    act(() => canvas.onClick!(gatePick, {}));

    expect(host.textContent).toContain("ICAO B");
    expect(host.textContent).toContain("AAL, DAL");
  });

  /** A read-only user still must not reach the editor — the card replaces it, not supplements it. */
  it("does not open the editor for a read-only user", () => {
    const host = mount(false);

    act(() => canvas.onClick!(gatePick, {}));

    expect(host.querySelector("input")).toBeNull();
    expect(host.textContent).not.toContain("Save");
  });

  /** An editor keeps the editor: that is what holding the permission buys. */
  it("opens the editor instead for a user who can edit", () => {
    const host = mount(true);

    act(() => canvas.onClick!(gatePick, {}));

    expect(host.querySelector("input"), "the editor's name field").not.toBeNull();
  });

  /** Clicking empty map must not leave a stale card open. */
  it("ignores a click that picked nothing", () => {
    const host = mount(false);

    act(() => canvas.onClick!({coordinate: [-77, 38]}, {}));

    expect(host.textContent).not.toContain("E57");
  });

  /** The card is dismissable, through the real handler's state rather than a prop. */
  it("closes the card again", () => {
    const host = mount(false);
    act(() => canvas.onClick!(gatePick, {}));

    const btn = host.querySelector<HTMLButtonElement>('[aria-label="Close stand details"]');
    act(() => btn!.click());

    expect(host.textContent).not.toContain("ICAO B");
  });
});

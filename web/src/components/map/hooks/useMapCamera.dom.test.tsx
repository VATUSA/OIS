// @vitest-environment jsdom
//
// VATUSA/OIS#540 AC2. `useMapCamera` seeded its canvas size to a hardcoded 800x600 and only
// corrected it from deck's `onResize` — which fires *after* effects. So any caller fitting bounds
// during a mount effect solved against the placeholder and got the wrong zoom and aspect, while a
// comment in the hook claimed the opposite.
//
// Six files call `fitBounds`, so this is tested at the hook rather than through one of them.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {useMapCamera, type MapCamera} from "./useMapCamera";
import {US_HOME} from "../lib/constants";

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

/** The live camera, plus a hook to fit during the very first effect — the broken case. */
function mountCamera(fitOnMount?: [number, number][]) {
  const seen: {camera?: MapCamera; viewState?: ReturnType<typeof useMapCamera>["viewState"]} = {};

  function Probe() {
    const camera = useMapCamera();
    seen.camera = camera;
    seen.viewState = camera.viewState;
    // Fires on the first effect and once only — the exact timing the bug needed, because
    // `onResize` has not fired by then. Guarded by a ref rather than an empty dependency array so
    // the real `exhaustive-deps` rule still applies to this fixture.
    const fitted = React.useRef(false);
    React.useEffect(() => {
      if (!fitOnMount || fitted.current) return;
      fitted.current = true;
      camera.fitBounds(fitOnMount, {padding: 60, maxZoom: 16});
    }, [camera]);
    return null;
  }

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  act(() => root.render(<Probe />));
  return seen;
}

/** A bounds around KDCA, as [lon, lat] — the order `fitBounds` takes. */
const DCA_BOUNDS: [number, number][] = [
  [-77.05, 38.84],
  [-77.03, 38.86],
];

describe("useMapCamera deferred fit (VATUSA/OIS#540)", () => {
  /**
   * The regression. A fit during a mount effect must not be solved against the placeholder size —
   * so it is held, and nothing moves until the canvas reports.
   */
  it("holds a fit requested before the canvas has reported its size", () => {
    const seen = mountCamera(DCA_BOUNDS);

    expect(seen.viewState?.zoom).toBe(US_HOME.zoom);
    expect(seen.viewState?.latitude).toBe(US_HOME.latitude);
  });

  /** …and then applies it, once a real size arrives. */
  it("applies the held fit when onResize lands", () => {
    const seen = mountCamera(DCA_BOUNDS);

    act(() => seen.camera!.onResize({width: 1400, height: 900}));

    expect(seen.viewState?.zoom).not.toBe(US_HOME.zoom);
    expect(seen.viewState?.latitude).toBeCloseTo(38.85, 1);
    expect(seen.viewState?.longitude).toBeCloseTo(-77.04, 1);
  });

  /**
   * The size actually used has to be the reported one, not the placeholder — that is the whole
   * point. A wide canvas and a tall one fit the same bounds at different zooms, so if both solved
   * against 800x600 these would match.
   */
  it("solves against the reported canvas, not the placeholder", () => {
    const wide = mountCamera(DCA_BOUNDS);
    act(() => wide.camera!.onResize({width: 2000, height: 400}));

    const tall = mountCamera(DCA_BOUNDS);
    act(() => tall.camera!.onResize({width: 400, height: 2000}));

    expect(wide.viewState?.zoom).not.toBeCloseTo(tall.viewState!.zoom!, 3);
  });

  /** A fit after the canvas is known must still apply immediately — no deferral once sized. */
  it("applies a later fit straight away", () => {
    const seen = mountCamera();

    act(() => seen.camera!.onResize({width: 1400, height: 900}));
    expect(seen.viewState?.zoom).toBe(US_HOME.zoom); // nothing asked for yet

    act(() => seen.camera!.fitBounds(DCA_BOUNDS, {maxZoom: 16}));
    expect(seen.viewState?.zoom).not.toBe(US_HOME.zoom);
  });

  /** A zero-sized resize is deck reporting an unlaid-out canvas; it must not unblock a fit. */
  it("ignores a zero-sized resize", () => {
    const seen = mountCamera(DCA_BOUNDS);

    act(() => seen.camera!.onResize({width: 0, height: 0}));

    expect(seen.viewState?.zoom).toBe(US_HOME.zoom);
  });

  /** Only one fit is held, and replaying it must not then fire on every later resize. */
  it("replays a held fit once, not on every resize", () => {
    const seen = mountCamera(DCA_BOUNDS);
    act(() => seen.camera!.onResize({width: 1400, height: 900}));
    const afterFirst = seen.viewState?.zoom;

    act(() => seen.camera!.onResize({width: 600, height: 600}));

    expect(seen.viewState?.zoom).toBe(afterFirst);
  });
});

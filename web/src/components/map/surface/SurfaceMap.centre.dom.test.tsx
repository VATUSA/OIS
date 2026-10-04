// @vitest-environment jsdom
//
// VATUSA/OIS#540 AC3 and AC4. The viewer used to fit to whatever geometry happened to be loaded,
// so an airport with none stayed on the CONUS view and you hunted for the field by hand. The FAA
// extract covers 185 fields, so "no geometry" is the common case rather than an edge one.
//
// The camera is captured from `MapCanvas`'s `viewState` prop, so these assert what the map was
// actually told to show.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {US_HOME} from "../lib/constants";
import type {AirportSurface} from "@/lib/airport-surface";

/** Every `viewState` MapCanvas has been handed, newest last. */
const cameras = vi.hoisted(() => ({seen: [] as {longitude: number; latitude: number; zoom: number}[]}));

vi.mock("../MapCanvas", () => ({
  MapCanvas: (props: {viewState?: {longitude: number; latitude: number; zoom: number}}) => {
    if (props.viewState) cameras.seen.push(props.viewState);
    return null;
  },
}));
vi.mock("../lib/palette", () => ({useMapPalette: () => ({ink: [255, 255, 255], line: [0, 0, 0]})}));
// `@ois/ui` renders fine under jsdom, so it is left real rather than stubbed export by export.

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
  cameras.seen.length = 0;
  vi.unstubAllGlobals();
});

/** A brand-new airport: no gates, no taxiways, no runways, no ramps. The common case. */
const EMPTY: AirportSurface = {
  gates: [],
  taxiways: [],
  runways: [],
  ramp_areas: [],
} as unknown as AirportSurface;

const DCA = {icao: "KDCA", lat: 38.8512, lon: -77.0402, elevation_ft: 15};

/** Seeds the position query directly, so no `fetch` is involved. */
// `null` means "no position known" — not `undefined`, which would select the default below.
async function mount(icao: string, position: typeof DCA | null = DCA) {
  const qc = new QueryClient({defaultOptions: {queries: {retry: false, refetchInterval: false}}});
  if (position) qc.setQueryData(["airport-position", icao], position);

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <SurfaceMap icao={icao} surface={EMPTY} editable={false} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {qc, root, host};
}

const latest = () => cameras.seen[cameras.seen.length - 1];

describe("SurfaceMap centring (VATUSA/OIS#540)", () => {
  /**
   * AC4, stated as the issue asks: an airport with **no surface geometry at all** must still centre,
   * so the camera has to end up somewhere other than `US_HOME`.
   */
  it("centres on an airport that has no geometry", async () => {
    await mount("KDCA");

    expect(latest().latitude).toBeCloseTo(DCA.lat, 2);
    expect(latest().longitude).toBeCloseTo(DCA.lon, 2);
    expect(latest().zoom).not.toBe(US_HOME.zoom);
  });

  /** The regression, in the terms the report used: it should not sit on the whole-US view. */
  it("does not leave the map on the continental view", async () => {
    await mount("KDCA");

    expect(latest().latitude).not.toBeCloseTo(US_HOME.latitude, 1);
    expect(latest().longitude).not.toBeCloseTo(US_HOME.longitude, 1);
  });

  /**
   * With no position available either, the camera must be left alone rather than flown somewhere
   * arbitrary — an airport missing from the dataset should look unmoved, not wrong.
   */
  it("leaves the camera alone when neither geometry nor a position is known", async () => {
    await mount("ZZZZ", null);

    expect(latest().latitude).toBeCloseTo(US_HOME.latitude, 5);
    expect(latest().zoom).toBe(US_HOME.zoom);
  });

  /**
   * AC3. The page remounts the viewer on `icao` change today, so this guards the *mechanism* rather
   * than the current wiring: the old `centered.current` boolean fired once per mount and would
   * silently stop re-centring the moment that page stopped remounting.
   */
  it("re-centres when the airport changes", async () => {
    const {qc, root} = await mount("KDCA");
    const first = latest();

    const sfo = {icao: "KSFO", lat: 37.6188, lon: -122.375, elevation_ft: 13};
    qc.setQueryData(["airport-position", "KSFO"], sfo);
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <ToastProvider>
            <SurfaceMap icao="KSFO" surface={EMPTY} editable={false} />
          </ToastProvider>
        </QueryClientProvider>,
      );
    });

    expect(latest().latitude).toBeCloseTo(sfo.lat, 2);
    expect(latest().latitude).not.toBeCloseTo(first.latitude, 2);
  });
});

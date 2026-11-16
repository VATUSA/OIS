// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6 on the airport arrival ladder: a tag whose time rests on the flight's proposed
// wheels-up is marked, in the classic style and in TGUI's.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {Flow, FlowFlight} from "@/lib/feed";
import {airportLadderItems, airportTguiColumns, LadderView} from "./airport";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

/** `min` minutes from the real clock: `LadderView` reads `Date.now()` itself. */
const inMin = (min: number) => new Date(Date.now() + min * 60_000).toISOString();

const row = (callsign: string, eta: number, over: Partial<FlowFlight>) =>
  ({
    callsign,
    dep: "KJFK",
    aircraft_type: "B738",
    category: "M",
    gate: null,
    status: "ground",
    distance_nm: 200,
    eta: inMin(eta),
    etd: inMin(eta - 40),
    groundspeed: 0,
    excluded: false,
    sta: null,
    delay_min: 0,
    seq: null,
    cfr: null,
    cfr_issued: false,
    wheels_up_proposed: false,
    ...over,
  }) as FlowFlight;

const flow = () =>
  ({
    icao: "KDCA",
    aar: null,
    flights: [row("GRD1", 20, { wheels_up_proposed: true }), row("ISS1", 35, { cfr_issued: true })],
    stars: [],
  }) as unknown as Flow;

function mount(style: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnMount: false, staleTime: Infinity } },
  });
  qc.setQueryData(["preferences", "settings"], { "ladder.style": style });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <LadderView flow={flow()} />
      </QueryClientProvider>,
    ),
  );
  return host;
}

/** The smallest element whose text holds `callsign`: its tag. */
function tag(host: HTMLElement, callsign: string, selector: string) {
  const found = [...host.querySelectorAll<HTMLElement>(selector)].filter((el) =>
    el.textContent?.includes(callsign),
  );
  expect(found.length, `${callsign} has a tag`).toBeGreaterThan(0);
  return found.at(-1)!;
}

describe("airport LadderView proposed marker (#814)", () => {
  it("marks a proposed flight's tag on the classic ladder", () => {
    const host = mount("classic");
    expect(host.querySelector('[data-ladder="tgui"]')).toBeNull();
    const grd = tag(host, "GRD1", "span.rounded-xs");
    expect(grd.querySelector(".sr-only")?.textContent).toBe(" proposed");
    expect(tag(host, "ISS1", "span.rounded-xs").querySelector(".sr-only")).toBeNull();
  });

  it("marks a proposed flight on the TGUI ladder", () => {
    const now = Date.now();
    const items = airportTguiColumns(airportLadderItems(flow(), undefined, now), now).flatMap((c) => c.items);
    expect(items.map((i) => [i.callsign, i.proposed])).toEqual([
      ["GRD1", true],
      ["ISS1", false],
    ]);
    const host = mount("tgui");
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    const grd = host.querySelectorAll('[data-tag][data-key="GRD1"]');
    expect(grd.length, "positive control: GRD1 is drawn").toBeGreaterThan(0);
    for (const t of grd) expect(t.querySelector(".sr-only")?.textContent).toBe(" proposed");
    for (const t of host.querySelectorAll('[data-tag][data-key="ISS1"]')) expect(t.querySelector(".sr-only")).toBeNull();
  });
});

describe("airport ladder saved status filter (#814)", () => {
  it("ignores a saved status the ladder no longer offers, and still applies the ones it does", () => {
    const now = Date.now();
    const shown = (statuses: string[]) =>
      airportLadderItems(flow(), { statuses }, now).map((i) => i.key);
    expect(shown(["proposed"])).toEqual(["GRD1", "ISS1"]);
    expect(shown(["ground", "proposed"])).toEqual(["GRD1", "ISS1"]);
    expect(shown(["airborne"]), "positive control: a live status still filters").toEqual([]);
  });
});

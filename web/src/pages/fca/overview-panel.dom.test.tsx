// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6 on the ARTCC overview's strips: a crossing that rests on the flight's proposed
// wheels-up is marked, and one held by a release is not.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {Fca, FcaFlight} from "@/lib/fca";
import {FcaOverviewPanel} from "./overview-panel";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  })) as unknown as typeof matchMedia;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const flight = (callsign: string, over: Partial<FcaFlight>) =>
  ({
    callsign,
    dep: "KJFK",
    arr: "KDCA",
    aircraft_type: "B738",
    status: "ground",
    seq: 1,
    cross_time: "2026-10-06T12:40:00Z",
    eta: "2026-10-06T12:40:00Z",
    delay_sec: 0,
    released: false,
    wheels_up_proposed: false,
    ...over,
  }) as FcaFlight;

const FCA = { id: "f1", name: "FCA1", color: "#33aaff", mode: "rate", rate: 30, mit: 0 } as unknown as Fca;

function mount(flights: FcaFlight[]) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() => root.render(<FcaOverviewPanel artcc="ZDC" groups={[{ fca: FCA, flights }]} />));
  return host;
}

/** Whether `callsign`'s strip shows its crossing marked proposed. */
function marked(host: HTMLElement, callsign: string) {
  const tr = [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes(callsign));
  expect(tr, `${callsign} is listed`).toBeDefined();
  return tr!.querySelector(".sr-only")?.textContent === " proposed";
}

describe("FcaOverviewPanel proposed crossing (#814)", () => {
  it("marks only the crossing that rests on a proposed wheels-up", () => {
    const host = mount([
      flight("GRD1", { wheels_up_proposed: true }),
      flight("REL1", { released: true, seq: 2 }),
    ]);
    expect(marked(host, "GRD1")).toBe(true);
    expect(marked(host, "REL1")).toBe(false);
  });
});

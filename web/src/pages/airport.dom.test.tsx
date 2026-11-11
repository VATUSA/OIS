// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6 on the airport page's aircraft table (`AircraftView`): an ETA that rests on a ground
// flight's proposed wheels-up is marked, and an ETA that does not is not.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {Flow, FlowFlight} from "@/lib/feed";
import {AircraftView, SummaryView} from "./airport";

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

const row = (callsign: string, over: Partial<FlowFlight>) =>
  ({
    callsign,
    dep: "KJFK",
    aircraft_type: "B738",
    category: "M",
    gate: null,
    status: "ground",
    distance_nm: 200,
    eta: "2026-10-06T12:40:00Z",
    etd: "2026-10-06T12:00:00Z",
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

function mount(flights: FlowFlight[]) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  qc.setQueryData(["preferences", "settings"], {});
  const flow = { icao: "KDCA", aar: 30, flights, stars: [] } as unknown as Flow;
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <AircraftView flow={flow} />
      </QueryClientProvider>,
    ),
  );
  return host;
}

/** Whether `callsign`'s row shows a time marked proposed. */
function marked(host: HTMLElement, callsign: string) {
  const tr = [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes(callsign));
  expect(tr, `${callsign} is listed`).toBeDefined();
  return tr!.querySelector(".sr-only")?.textContent === " proposed";
}

describe("AircraftView proposed ETA (#814)", () => {
  it("calls an unissued CFR metered and an issued one locked (decision 33)", () => {
    const host = mount([
      row("MET", { cfr: "2026-10-06T12:05:00Z" }),
      row("ISS", { cfr: "2026-10-06T12:05:00Z", cfr_issued: true }),
    ]);
    expect(host.querySelector('[title^="Metered wheels-up"]')?.textContent).toBe("1205z");
    expect(host.querySelector('[title^="Call-for-release issued"]')?.textContent).toBe("1205z");
    expect(host.textContent).toContain("faint times are metered slots");
  });

  it("marks the ETA of a ground flight with no lock, and nothing else", () => {
    const host = mount([
      row("GRD", { wheels_up_proposed: true }),
      row("ISS", { cfr: "2026-10-06T12:05:00Z", cfr_issued: true }),
      row("AIR", { status: "airborne", etd: null, groundspeed: 450 }),
    ]);
    expect(marked(host, "GRD")).toBe(true);
    expect(marked(host, "ISS")).toBe(false);
    expect(marked(host, "AIR")).toBe(false);
    const grd = [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes("GRD"));
    expect(grd?.textContent).toContain("1240zP proposed");
  });
});

describe("SummaryView status cards (#814)", () => {
  it("counts airborne, ground and arrived, with no Proposed card now prefiles are not listed", () => {
    const flow = {
      icao: "KDCA",
      aar: 30,
      inbound: 3,
      airborne: 1,
      ground: 2,
      demand_60min: 0,
      over_capacity: false,
      flights: [
        row("AIR", { status: "airborne", etd: null, groundspeed: 450 }),
        row("GRD1", {}),
        row("GRD2", {}),
        row("ARR", { status: "arrived", etd: null }),
      ],
      stars: [],
    } as unknown as Flow;
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
    qc.setQueryData(["preferences", "settings"], {});
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({ root, host });
    act(() =>
      root.render(
        <QueryClientProvider client={qc}>
          <SummaryView flow={flow} />
        </QueryClientProvider>,
      ),
    );
    const card = (label: string) =>
      [...host.querySelectorAll<HTMLElement>("div.rounded-md")].find((c) => c.textContent?.startsWith(label));
    for (const [label, count] of [
      ["Inbound total", "3"],
      ["Airborne", "1"],
      ["Ground", "2"],
      ["Arrived", "1"],
    ]) {
      expect(card(label)?.textContent, label).toBe(`${label}${count}`);
    }
    expect(host.textContent).not.toContain("Proposed");
  });
});

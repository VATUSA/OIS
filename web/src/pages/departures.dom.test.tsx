// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6 on the departures list (`DeparturesView`): an ETA that rests on a proposed wheels-up
// is marked, and a locked departure's is not.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// Every query the view reads is seeded below; nothing reaches the API.
const api = vi.hoisted(() => ({
  GET: vi.fn(() => new Promise(() => {})),
  POST: vi.fn(() => new Promise(() => {})),
  PUT: vi.fn(() => new Promise(() => {})),
  DELETE: vi.fn(() => new Promise(() => {})),
}));
vi.mock("@/lib/api", () => ({ois: api, API_BASE: "", DOCS_URL: ""}));

import type {Me} from "@/lib/auth";
import type {Departure, DeparturesResponse} from "@/lib/departures";
import {DeparturesView} from "./departures";

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
});

const ME = { server_admin: false, permissions: { tmu: { cfr: ["assign"], program: ["read"] } } } as unknown as Me;

const departure = (callsign: string, over: Partial<Departure>) =>
  ({
    callsign,
    dep: "KJFK",
    arrival: "KDCA",
    aircraft_type: "B738",
    gate: null,
    status: "ground",
    has_program: true,
    eta: "2026-10-06T12:40:00Z",
    sta: null,
    delay_min: 0,
    cfr: null,
    cfr_issued: false,
    wheels_up_proposed: false,
    cfr_version: null,
    seq: null,
    ...over,
  }) as Departure;

function mount(departures: Departure[]) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnMount: false, refetchOnWindowFocus: false, refetchOnReconnect: false },
    },
  });
  qc.setQueryData(["me"], ME);
  qc.setQueryData(["preferences", "settings"], {});
  qc.setQueryData(["departures", "KJFK"], {
    airports: ["KJFK"],
    departures,
    facility_kind: null,
    holding_on_cfr: 0,
    program_destinations: ["KDCA"],
    to_metered: departures.length,
    total: departures.length,
  } as unknown as DeparturesResponse);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <DeparturesView icao="KJFK" />
        </ToastProvider>
      </QueryClientProvider>,
    ),
  );
  return host;
}

function marked(host: HTMLElement, callsign: string) {
  const tr = [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes(callsign));
  expect(tr, `${callsign} is listed`).toBeDefined();
  return tr!.querySelector(".sr-only")?.textContent === " proposed";
}

describe("DeparturesView proposed ETA (#814)", () => {
  it("marks the ETA of a departure nothing locks, and not a locked one", () => {
    const host = mount([
      departure("FREE", { wheels_up_proposed: true, cfr: "2026-10-06T12:05:00Z" }),
      departure("CFR", { cfr: "2026-10-06T12:10:00Z", cfr_issued: true }),
    ]);
    expect(marked(host, "FREE")).toBe(true);
    expect(marked(host, "CFR")).toBe(false);
    const free = [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes("FREE"));
    expect(free?.textContent).toContain("1240zP proposed");
  });

  it("calls an unissued CFR metered and an issued one issued (decision 33)", () => {
    const host = mount([
      departure("FREE", { cfr: "2026-10-06T12:05:00Z" }),
      departure("LCK1", { cfr: "2026-10-06T12:10:00Z", cfr_issued: true }),
    ]);
    const row = (cs: string) => [...host.querySelectorAll("tr")].find((r) => r.textContent?.includes(cs));
    expect(row("FREE")?.textContent).toContain("metered");
    expect(row("LCK1")?.textContent).toContain("issued");
  });
});

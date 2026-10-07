// @vitest-environment jsdom
//
// VATUSA/OIS#746. Release, clear and reorder answer 404 on an event FCA that is planned or archived to
// anyone but an event planner (as #736 gates edit/delete). A planner keeps them in the event builder,
// where they sequence a planned FCA; a `flow.fca.*` holder who reaches one through a stale selection
// on the ops map must not be offered controls that can only fail.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

// Every query the panel reads is seeded below; only the writes a click makes reach the API.
const api = vi.hoisted(() => ({
  GET: vi.fn(() => new Promise(() => {})),
  POST: vi.fn(() => new Promise(() => {})),
  PUT: vi.fn(() => new Promise(() => {})),
  DELETE: vi.fn(() => new Promise(() => {})),
}));
vi.mock("@/lib/api", () => ({ois: api, API_BASE: "", DOCS_URL: ""}));
// The ladder is pure presentation with its own settings reads; it plays no part in the controls.
vi.mock("@/pages/fca/ladder", () => ({Ladder: () => null}));

import type {Me} from "@/lib/auth";
import type {Fca, FcaFlight} from "@/lib/fca";
import {exclusionsKey} from "@/lib/flight-exclusions";
import {FcaDetail} from "./detail";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom has no `matchMedia`; the panel's `Sheet` asks it whether it is on a phone. Desktop here.
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

beforeEach(() => {
  for (const fn of [api.POST, api.PUT, api.DELETE]) fn.mockClear();
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const FLOW = { flow: { fca: ["read", "update", "delete"] } };
/** A rostered controller on the ops map: `flow.fca.*`, no event planning. */
const CONTROLLER = { server_admin: false, permissions: FLOW } as unknown as Me;
/** Can see the event builder but not plan: the routes still refuse them. */
const PLAN_READER = {
  server_admin: false,
  permissions: { ...FLOW, events: { plan: ["read"] } },
} as unknown as Me;
const PLANNER = {
  server_admin: false,
  permissions: { ...FLOW, events: { plan: ["read", "update"] } },
} as unknown as Me;

const fca = (eventId: number | null, eventStatus: string | null) =>
  ({
    id: "f-1",
    name: "FCA1",
    color: "#efc14d",
    artcc: "ZDC",
    enabled: true,
    mode: "rate",
    rate: 30,
    mit: 0,
    manual_seq: true,
    event_id: eventId,
    event_status: eventStatus,
  }) as unknown as Fca;

const flight = (callsign: string, released: boolean) =>
  ({
    callsign,
    aircraft_type: "B738",
    altitude: 0,
    dep: "KIAD",
    arr: "KATL",
    cross_lat: 0,
    cross_lon: 0,
    cross_time: "2026-10-06T12:00:00Z",
    delay_min: 0,
    delay_nm: 0,
    delay_sec: 0,
    distance_nm: 120,
    released,
    edct: released ? "2026-10-06T11:40:00Z" : null,
    seq: released ? 2 : 1,
    status: "ground",
  }) as unknown as FcaFlight;

const FLIGHTS = [flight("AAL1", false), flight("DAL2", true)];

/** Mounts the panel as `FcaMapView` would: `canEdit` is the map's own gate (`flow.fca.update` on the
 *  ops map, `events.plan.update` in the event builder), `me` seeded in the real query cache. */
function mount(me: Me, row: Fca, canEdit: boolean) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(["me"], me);
  qc.setQueryData(exclusionsKey(row.id), { exclusions: [], editable: false });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <FcaDetail fca={row} flights={FLIGHTS} canEdit={canEdit} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

/** Which live-operation controls the panel offers. */
function controls(host: HTMLElement) {
  const buttons = [...host.querySelectorAll("button")];
  return {
    release: buttons.some((b) => b.textContent === "RDY"),
    clear: buttons.some((b) => b.getAttribute("aria-label") === "Clear release"),
    reorder: buttons.some((b) => b.getAttribute("aria-label")?.startsWith("Reorder ")),
    reset: buttons.some((b) => b.title === "Reset to automatic sequencing"),
  };
}

const ALL = { release: true, clear: true, reorder: true, reset: true };
const NONE = { release: false, clear: false, reorder: false, reset: false };

const click = (host: HTMLElement, find: (b: HTMLButtonElement) => boolean) =>
  act(async () => [...host.querySelectorAll("button")].find(find)!.click());

describe("FcaDetail live-operation controls (VATUSA/OIS#746)", () => {
  it.each([
    ["planned", "planned"],
    ["archived", "archived"],
  ])("a planner keeps them on a %s event FCA, and they reach the API", async (_l, status) => {
    const host = mount(PLANNER, fca(7461, status), true);
    expect(controls(host)).toEqual(ALL);
    await click(host, (b) => b.textContent === "RDY");
    expect(api.POST).toHaveBeenCalledWith("/api/v1/flow/fcas/{id}/release/{callsign}", expect.anything());
    await click(host, (b) => b.getAttribute("aria-label") === "Clear release");
    expect(api.DELETE).toHaveBeenCalledWith("/api/v1/flow/fcas/{id}/release/{callsign}", expect.anything());
    await click(host, (b) => b.title === "Reset to automatic sequencing");
    expect(api.PUT).toHaveBeenCalledWith("/api/v1/flow/fcas/{id}/order", expect.objectContaining({ body: { order: [] } }));
  });

  it.each([
    ["a controller", CONTROLLER, "planned"],
    ["a controller", CONTROLLER, "archived"],
    ["a plan reader", PLAN_READER, "planned"],
  ])("%s holding flow.fca.update gets none on a %s event FCA", (_l, me, status) => {
    const host = mount(me, fca(7461, status), true);
    expect(controls(host)).toEqual(NONE);
    // The sequence is still marked manual — as a label, not a control.
    expect(host.textContent).toContain("manual");
  });

  it.each([
    ["a published event FCA", 7461, "published"],
    ["an ordinary FCA", null, null],
  ])("a controller keeps them on %s", (_l, id, status) => {
    expect(controls(mount(CONTROLLER, fca(id, status), true))).toEqual(ALL);
  });

  it("offers none to a caller who can't edit here, planner or not", () => {
    expect(controls(mount(PLANNER, fca(7461, "planned"), false))).toEqual(NONE);
    expect(controls(mount(CONTROLLER, fca(null, null), false))).toEqual(NONE);
  });
});

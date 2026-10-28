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

// VATUSA/OIS#819: the gap on each strip and the two-aircraft compare panel.
describe("FcaDetail gap and compare panel (VATUSA/OIS#819)", () => {
  /** A strip with every field the gap readout and the compare panel show. */
  const strip = (
    callsign: string,
    seq: number,
    o: {
      eta: string;
      cross: string;
      placed: boolean;
      gap: [number, number] | null;
      speed?: number;
      distance?: number;
      req?: [number, number];
    },
  ) =>
    ({
      callsign,
      aircraft_type: "B738",
      altitude: 30000,
      dep: "KJFK",
      arr: "KDCA",
      cross_lat: 0,
      cross_lon: 0,
      eta: o.eta,
      cross_time: o.cross,
      delay_min: 0,
      delay_nm: 0,
      delay_sec: 0,
      distance_nm: o.distance ?? 100,
      released: false,
      edct: null,
      seq,
      status: "airborne",
      gap_to_leader_sec: o.gap?.[0] ?? null,
      gap_to_leader_nm: o.gap?.[1] ?? null,
      required_spacing_sec: o.req?.[0] ?? 120,
      required_spacing_nm: o.req?.[1] ?? 10,
      cross_speed: o.speed ?? 300,
      manually_placed: o.placed,
    }) as unknown as FcaFlight;

  // The controller put DAL2 (later ETA) ahead of AAL1; UAL3 follows by ETA.
  const MANUAL = [
    strip("DAL2", 1, {eta: "2026-10-27T12:10:00Z", cross: "2026-10-27T12:10:00Z", placed: true, gap: null, speed: 320, distance: 140, req: [150, 13.3]}),
    strip("AAL1", 2, {eta: "2026-10-27T12:05:00Z", cross: "2026-10-27T12:12:00Z", placed: true, gap: [120, 10]}),
    strip("UAL3", 3, {eta: "2026-10-27T12:20:00Z", cross: "2026-10-27T12:20:00Z", placed: false, gap: [480, 40]}),
  ];

  function mount(flights: FcaFlight[], canEdit = true) {
    const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
    qc.setQueryData(["me"], CONTROLLER);
    qc.setQueryData(exclusionsKey("f-1"), {exclusions: [], editable: false});
    qc.setQueryData(exclusionsKey("f-2"), {exclusions: [], editable: false});
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    const render = (list: FcaFlight[], fcaId = "f-1") =>
      act(() => {
        root.render(
          <QueryClientProvider client={qc}>
            <ToastProvider>
              <FcaDetail fca={{...fca(null, null), id: fcaId}} flights={list} canEdit={canEdit} />
            </ToastProvider>
          </QueryClientProvider>,
        );
      });
    render(flights);
    return {host, render};
  }
  const mountWith = (flights: FcaFlight[], canEdit = true) => mount(flights, canEdit).host;

  const toggle = (host: HTMLElement, cs: string) =>
    host.querySelector<HTMLButtonElement>(`button[aria-label="Compare ${cs}"]`)!;
  const pick = (host: HTMLElement, cs: string) => act(() => toggle(host, cs).click());
  const panel = (host: HTMLElement) => host.querySelector('section[aria-label="Compare aircraft"]');
  const text = (host: HTMLElement, id: string) =>
    host.querySelector(`[data-testid="${id}"]`)?.textContent;
  const stripOf = (host: HTMLElement, cs: string) => toggle(host, cs).closest("li")!;

  it("shows each strip's gap to the aircraft ahead in seconds and nm", () => {
    const host = mountWith(MANUAL);
    expect(stripOf(host, "AAL1").textContent).toContain("gap 2:00 · 10.0nm");
    expect(stripOf(host, "UAL3").textContent).toContain("gap 8:00 · 40.0nm");
  });

  it("shows no gap on the first strip in the sequence", () => {
    const host = mountWith(MANUAL);
    expect(stripOf(host, "DAL2").textContent).not.toContain("gap ");
    expect(stripOf(host, "AAL1").textContent).toContain("gap ");
  });

  it("marks a strip's compare toggle pressed when clicked, and unpressed when clicked again", () => {
    const host = mountWith(MANUAL);
    expect(toggle(host, "AAL1").getAttribute("aria-pressed")).toBe("false");
    pick(host, "AAL1");
    expect(toggle(host, "AAL1").getAttribute("aria-pressed")).toBe("true");
    pick(host, "AAL1");
    expect(toggle(host, "AAL1").getAttribute("aria-pressed")).toBe("false");
  });

  it("keeps at most two flights selected for comparison", () => {
    const host = mountWith(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    expect(toggle(host, "UAL3").disabled).toBe(true);
    pick(host, "UAL3");
    const pressed = MANUAL.filter((f) => toggle(host, f.callsign).getAttribute("aria-pressed") === "true");
    expect(pressed.map((f) => f.callsign)).toEqual(["DAL2", "AAL1"]);
  });

  it("shows both aircraft's crossing times, crossing speed, distance to line, gap and required spacing", () => {
    const host = mountWith(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    const p = panel(host)!;
    expect(p.textContent).toContain("1210");
    expect(p.textContent).toContain("1212");
    expect(p.textContent).toContain("320kt");
    expect(p.textContent).toContain("300kt");
    expect(p.textContent).toContain("140nm");
    expect(p.textContent).toContain("100nm");
    // 12:12 − 12:10 = 120 s, at AAL1's 300 kt = 10 nm.
    expect(text(host, "compare-gap")).toBe("2:00 · 10.0nm");
    // The required spacing is the behind flight's (AAL1's 120 s), not DAL2's 150 s.
    expect(text(host, "compare-required")).toBe("2:00 · 10.0nm");
  });

  it("closes the compare panel and frees both toggles from its close button", () => {
    const host = mountWith(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="Close compare"]')!.click());
    expect(panel(host)).toBeNull();
    expect(["DAL2", "AAL1", "UAL3"].map((cs) => toggle(host, cs).getAttribute("aria-pressed"))).toEqual(["false", "false", "false"]);
    expect(toggle(host, "UAL3").disabled).toBe(false);
  });

  it("puts the lower-sequence flight first whichever was selected first", () => {
    const host = mountWith(MANUAL);
    pick(host, "UAL3");
    pick(host, "DAL2");
    const heads = [...panel(host)!.querySelectorAll("th")].map((th) => th.textContent);
    expect(heads.slice(1)).toEqual(["DAL2", "UAL3"]);
  });

  // AC3's named DOM test.
  it("explains a pair the controller placed as manual placement", () => {
    const host = mountWith(MANUAL);
    pick(host, "AAL1");
    pick(host, "DAL2");
    expect(text(host, "compare-reason")).toBe("DAL2 is ahead of AAL1 by manual placement.");
  });

  it("explains a pair the scheduler pushed apart as a spacing push", () => {
    const auto = [
      strip("AIR1", 1, {eta: "2026-10-27T12:06:00Z", cross: "2026-10-27T12:06:00Z", placed: false, gap: null}),
      strip("GND2", 2, {eta: "2026-10-27T12:05:00Z", cross: "2026-10-27T12:08:00Z", placed: false, gap: [120, 10]}),
    ];
    const host = mountWith(auto);
    pick(host, "AIR1");
    pick(host, "GND2");
    expect(text(host, "compare-reason")).toBe(
      "GND2 has the earlier ETA but was pushed behind AIR1 to hold spacing.",
    );
  });

  it("hides the compare panel when one of the two is deselected", () => {
    const host = mountWith(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    expect(panel(host)).not.toBeNull();
    pick(host, "AAL1");
    expect(panel(host)).toBeNull();
  });

  it("offers the compare toggle to a caller who cannot edit", () => {
    const host = mountWith(MANUAL, false);
    pick(host, "DAL2");
    pick(host, "UAL3");
    expect(text(host, "compare-reason")).toBe("DAL2 is ahead of UAL3 in ETA order.");
  });

  it("frees a pick whose flight has left the list, so the other toggles work again", () => {
    const {host, render} = mount(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    expect(toggle(host, "UAL3").disabled).toBe(true);
    render(MANUAL.filter((f) => f.callsign !== "AAL1"));
    expect(panel(host)).toBeNull();
    expect(toggle(host, "UAL3").disabled).toBe(false);
    pick(host, "UAL3");
    expect(text(host, "compare-reason")).toBe("DAL2 is ahead of UAL3 in ETA order.");
    // AAL1 coming back does not revive its old pick as a third selection.
    render(MANUAL);
    expect(text(host, "compare-reason")).toBe("DAL2 is ahead of UAL3 in ETA order.");
    expect(toggle(host, "AAL1").getAttribute("aria-pressed")).toBe("false");
  });

  it("starts with nothing picked after switching to another FCA", () => {
    const {host, render} = mount(MANUAL);
    pick(host, "DAL2");
    pick(host, "AAL1");
    expect(panel(host)).not.toBeNull();
    render(MANUAL, "f-2");
    expect(panel(host)).toBeNull();
    expect(["DAL2", "AAL1", "UAL3"].map((cs) => toggle(host, cs).disabled)).toEqual([false, false, false]);
  });

  it("measures a non-adjacent pair's gap between the two of them, not the strip's gap", () => {
    // DAL2 (seq 1, 12:10) and UAL3 (seq 3, 12:20): 600 s at UAL3's 300 kt = 50 nm. UAL3's own strip
    // gap is to AAL1 (40 nm), which must not be shown for this pair.
    const host = mountWith(MANUAL);
    pick(host, "DAL2");
    pick(host, "UAL3");
    expect(text(host, "compare-gap")).toBe("10:00 · 50.0nm");
  });

  it("shows an adjacent pair's gap in nm as the strip reports it", () => {
    // 500 s at 283.4 kt is 39.4 nm on the backend; recomputed at the rounded 283 kt it would be 39.3.
    const pairOf = [
      strip("DAL2", 1, {eta: "2026-10-27T12:00:00Z", cross: "2026-10-27T12:00:00Z", placed: false, gap: null}),
      strip("AAL1", 2, {eta: "2026-10-27T12:08:20Z", cross: "2026-10-27T12:08:20Z", placed: false, gap: [500, 39.4], speed: 283}),
    ];
    const host = mountWith(pairOf);
    pick(host, "DAL2");
    pick(host, "AAL1");
    expect(text(host, "compare-gap")).toBe("8:20 · 39.4nm");
  });
});

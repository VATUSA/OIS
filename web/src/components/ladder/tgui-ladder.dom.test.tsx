// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {ViewWidgetView} from "@/features/dashboard/view-widgets";
import {WidgetStatusReporter} from "@/features/dashboard/widget-status";
import type {ViewWidget} from "@/features/dashboard/types";
import type {FcaFlight} from "@/lib/fca";
import type {Flow, FlowFlight} from "@/lib/feed";
import {Ladder, ladderItems, tguiColumn} from "@/pages/fca/ladder";
import {airportLadderItems, airportTguiColumns, LadderView} from "@/pages/airport";

import {TguiLadder} from "./TguiLadder";
import type {TguiColumn, TguiItem} from "./tgui";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom has no ResizeObserver, and the classic ladder measures its width with one. A no-op is
  // enough: an unmeasured ladder assumes unlimited width (its documented first-paint behaviour).
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

const NOW = Date.parse("2026-10-03T12:00:00Z");
const at = (min: number) => new Date(NOW + min * 60_000).toISOString();

/** A real QueryClient with the settings blob seeded — the real `useSetting` runs, nothing is
 * stubbed, and seeded data is never refetched. */
function client(settings: Record<string, unknown> | null, seed: [unknown[], unknown][] = []) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false, refetchOnMount: false, staleTime: Infinity } },
  });
  qc.setQueryData(["preferences", "settings"], settings);
  for (const [key, data] of seed) qc.setQueryData(key, data);
  return qc;
}

function mount(node: React.ReactNode, qc = client(null)) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() => root.render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>));
  return host;
}

const tguiItem = (key: string, staMin: number, over: Partial<TguiItem> = {}): TguiItem => ({
  key,
  callsign: key,
  etaMin: staMin,
  etaTime: at(staMin),
  staMin,
  staTime: at(staMin),
  delayMin: 0,
  committed: false,
  ...over,
});

const column = (items: TguiItem[]): TguiColumn => ({ id: "c", name: "BEARR", kind: "MFX", items });

function slots(host: HTMLElement, rail: "eta" | "sta") {
  return [...host.querySelectorAll<HTMLElement>(`[data-slot="${rail}"]`)];
}

describe("TguiLadder (VATUSA/OIS#557)", () => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });

  it("draws every flight twice: on the ETA rail and on the schedule rail", () => {
    const host = mount(<TguiLadder columns={[column([tguiItem("AAL1", 10), tguiItem("UAL2", 30)])]} now={NOW} win={60} emptyMessage="none" />);
    expect(host.querySelectorAll('[data-tag="eta"]')).toHaveLength(2);
    expect(host.querySelectorAll('[data-tag="sta"]')).toHaveLength(2);
  });

  it("puts aircraft sharing an exact time in one slot, and never widens the column for them", () => {
    const same = [1, 2, 3, 4, 5, 6].map((n) => tguiItem(`AAL${n}`, 20));
    const host = mount(<TguiLadder columns={[column(same)]} now={NOW} win={60} emptyMessage="none" />);
    expect(slots(host, "sta")).toHaveLength(1);
    // The column has a fixed width; whatever does not fit collapses to "+N" inside it.
    const col = host.querySelector<HTMLElement>("[data-column]")!;
    expect(col.style.width).toBe(`${132 * 2 + 40}px`);
    expect(slots(host, "sta")[0].textContent).toMatch(/\+\d/);
  });

  it("pushes a packed tag up, never down, and its leader still points at its true time", () => {
    // 6 px a minute: three flights 0.2 min apart are far closer than a row, so two are pushed.
    const dense = [tguiItem("AAL1", 10), tguiItem("AAL2", 10.2), tguiItem("AAL3", 10.4)];
    const host = mount(<TguiLadder columns={[column(dense)]} now={NOW} win={60} emptyMessage="none" />);
    const placed = slots(host, "sta");
    expect(placed).toHaveLength(3);
    for (const slot of placed) {
      // Up is a smaller y. A pushed tag sits at or above its true time — never below it.
      expect(Number(slot.dataset.y)).toBeLessThanOrEqual(Number(slot.dataset.trueY));
    }
    expect(Number(placed[2].dataset.y)).toBeLessThan(Number(placed[2].dataset.trueY));

    const leader = host.querySelector<SVGLineElement>('[data-leader="sta"][data-key="AAL3"]')!;
    expect(Number(leader.getAttribute("y2"))).toBe(Number(placed[2].dataset.trueY));
    expect(Number(leader.getAttribute("y1"))).toBe(Number(placed[2].dataset.y));
  });

  it("labels the delay beside the schedule tag only, and draws none when there is no delay to report", () => {
    const host = mount(
      <TguiLadder
        columns={[column([tguiItem("AAL1", 10, { delayMin: 7 }), tguiItem("UAL2", 20, { delayMin: null })])]}
        now={NOW}
        win={60}
        emptyMessage="none"
      />,
    );
    const label = host.querySelectorAll("[data-delay]");
    expect(label).toHaveLength(1);
    expect(label[0].textContent).toBe("07");
    expect(label[0].getAttribute("data-delay")).toBe("watch");
    expect(label[0].closest('[data-tag]')?.getAttribute("data-tag")).toBe("sta");
  });

  it("marks a committed time (a release or an issued CFR) on both rails", () => {
    const host = mount(<TguiLadder columns={[column([tguiItem("AAL1", 10, { committed: true })])]} now={NOW} win={60} emptyMessage="none" />);
    expect(host.querySelectorAll('[data-tag][data-committed="true"]')).toHaveLength(2);
  });

  it("captions a column NAME [n] NAME with its reference kind beneath", () => {
    const host = mount(<TguiLadder columns={[column([tguiItem("AAL1", 10)])]} now={NOW} win={60} emptyMessage="none" />);
    expect(host.textContent).toContain("BEARR1BEARR");
    expect(host.textContent).toContain("MFX");
  });
});

// ---- AC5: one sequence, two renderings -----------------------------------------------------------

const fca = (callsign: string, cross: number, over: Partial<FcaFlight> = {}) =>
  ({
    callsign,
    status: "airborne",
    seq: 0,
    cross_time: at(cross),
    eta: at(cross - 2),
    delay_sec: 120,
    released: false,
    ...over,
  }) as FcaFlight;

const arrival = (callsign: string, sta: number | null, gate: string | null, over: Partial<FlowFlight> = {}) =>
  ({
    callsign,
    status: "airborne",
    excluded: false,
    eta: at((sta ?? 30) - 3),
    sta: sta == null ? null : at(sta),
    gate,
    delay_min: 3,
    cfr_issued: false,
    category: "M",
    dep: "KJFK",
    aircraft_type: "B738",
    ...over,
  }) as FlowFlight;

describe("TGUI and classic show the same sequence (AC5)", () => {
  it("FCA: the schedule rail holds the classic ladder's flights, in its order, at its times", () => {
    const flights = [fca("C3", 40), fca("A1", 10), fca("PROP", 5, { status: "proposed" }), fca("B2", 25)];
    const classic = ladderItems(flights, NOW).sort((a, b) => a.min - b.min);
    const tgui = tguiColumn(flights, NOW, "X").items.sort((a, b) => a.staMin - b.staMin);
    expect(tgui.map((i) => [i.key, i.staTime])).toEqual(classic.map((i) => [i.key, i.time]));
  });

  it("airport: each gate column is the classic sequence restricted to that gate, and nothing is lost", () => {
    const flow = {
      icao: "KDCA",
      flights: [
        arrival("A1", 10, "BEARR"),
        arrival("B1", 12, "CORIN"),
        arrival("A2", 20, "BEARR"),
        arrival("X1", 15, null),
        arrival("GONE", 8, "BEARR", { status: "arrived" }),
        arrival("NOSTA", null, "CORIN"), // no AAR: plotted at its ETA, as the classic ladder does
      ],
    } as unknown as Flow;
    const classic = airportLadderItems(flow, undefined, NOW).sort((a, b) => a.min - b.min);
    const columns = airportTguiColumns(airportLadderItems(flow, undefined, NOW), NOW);

    expect(columns.map((c) => c.name)).toEqual(["BEARR", "CORIN", "OTHER"]);
    for (const c of columns) {
      const expected = classic.filter((i) => (i.data.gate ?? "OTHER") === c.name).map((i) => [i.key, i.time]);
      expect(c.items.map((i) => [i.key, i.staTime])).toEqual(expected);
    }
    expect(columns.flatMap((c) => c.items).length).toBe(classic.length);
    expect(columns[1].items.find((i) => i.key === "NOSTA")?.delayMin).toBeNull();
  });
});

// ---- AC2: one setting reaches every ladder, the pop-out included ---------------------------------

describe("ladder.style switches every arrival ladder (AC2)", () => {
  const flights = [fca("AAL1", 10), fca("UAL2", 20)];
  const flow = { icao: "KDCA", aar: 40, flights: [arrival("A1", 10, "BEARR")] } as unknown as Flow;

  // `Ladder` is the component both the FCA detail panel and its pop-out window render, so the
  // pop-out follows the setting because the choice is made in here, not by either caller.
  it("TGUI: the FCA ladder (and so its pop-out) and the airport ladder both draw TGUI", () => {
    const fcaHost = mount(<Ladder flights={flights} now={NOW} name="ZDC WEST" />, client({ "ladder.style": "tgui" }));
    expect(fcaHost.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    expect(fcaHost.textContent).toContain("ZDC WEST1ZDC WEST");
    expect(fcaHost.textContent).toContain("FCA");

    const airportHost = mount(<LadderView flow={flow} />, client({ "ladder.style": "tgui" }));
    expect(airportHost.querySelector('[data-ladder="tgui"]')).not.toBeNull();
  });

  it("classic (or unset): both draw the classic ladder", () => {
    for (const settings of [{ "ladder.style": "classic" }, {}, null]) {
      const fcaHost = mount(<Ladder flights={flights} now={NOW} name="ZDC WEST" />, client(settings));
      expect(fcaHost.querySelector('[data-ladder="tgui"]')).toBeNull();
      expect(fcaHost.textContent).toContain("NOW");

      const airportHost = mount(<LadderView flow={flow} />, client(settings));
      expect(airportHost.querySelector('[data-ladder="tgui"]')).toBeNull();
      expect(airportHost.textContent).toContain("NOW");
    }
  });
});

// ---- AC1 + AC6: the widget ------------------------------------------------------------------------

describe("the TGUI dashboard widget", () => {
  const flow = { icao: "KDCA", aar: 40, flights: [arrival("A1", 10, "BEARR"), arrival("B1", 12, "CORIN")] };
  const widget = (view: ViewWidget["view"]): ViewWidget => ({ id: "w1", kind: "view", view, icao: "KDCA" });

  it("draws one TGUI column per arrival gate, whatever ladder.style says", () => {
    const host = mount(
      <ViewWidgetView widget={widget("airport-tgui")} editing={false} onChange={() => {}} />,
      client({ "ladder.style": "classic" }, [[["flow", "KDCA"], flow]]),
    );
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    expect([...host.querySelectorAll("[data-column]")].map((c) => c.getAttribute("data-column"))).toEqual([
      "BEARR",
      "CORIN",
    ]);
  });

  it.each(["airport-tgui", "airport-ladder"] as const)("%s reports its fetch status to the frame", (view) => {
    const report = vi.fn();
    mount(
      <WidgetStatusReporter value={report}>
        <ViewWidgetView widget={widget(view)} editing={false} onChange={() => {}} />
      </WidgetStatusReporter>,
      client(null, [[["flow", "KDCA"], flow]]),
    );
    const last = report.mock.calls.at(-1)?.[0];
    expect(last).toMatchObject({ isFetching: false });
    expect(last.updatedAt).toBeGreaterThan(0);
  });
});

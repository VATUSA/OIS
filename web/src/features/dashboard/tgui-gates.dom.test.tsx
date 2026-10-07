// @vitest-environment jsdom
import {readFileSync} from "node:fs";
import {resolve} from "node:path";

import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {Flow, FlowFlight} from "@/lib/feed";
import {airportLadderItems, airportTguiColumns, LadderView, summaryGateName} from "@/pages/airport";

import type {ViewWidget} from "./types";
import {ladderGateOptions, ViewWidgetView} from "./view-widgets";

// VATUSA/OIS#791 — TGUI gate choices come from two sources: the nav data's STARs for the airport
// (`Flow.stars`) and each live flight's `arrival_gate` (`FlowFlight.gate`). Both go through
// `summaryGateName`; if they ever grouped differently, a column configured from the static list would
// never fill. The fixture is shared with `backend/src/feed/flow.rs`'s tests, which prove the backend
// emits these exact strings, so a change on either side turns one of the two suites red.

interface Case {
  name: string;
  route: string;
  gate: string;
  star: string;
  column: string;
}

// Read rather than `import`: the fixture lives at the repo root, outside this package (see
// `web/src/lib/ntml.test.ts`). A path, not `import.meta.url`: under jsdom that URL is not `file:`.
const FIXTURE_PATH = resolve(__dirname, "../../../../fixtures/tgui-gate-names.json");
const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as { airport: string; stars: string[]; cases: Case[] };

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

const NOW = Date.parse("2026-10-07T12:00:00Z");
const at = (min: number) => new Date(NOW + min * 60_000).toISOString();

const arrival = (callsign: string, min: number, gate: string | null) =>
  ({
    callsign,
    status: "airborne",
    excluded: false,
    eta: at(min),
    sta: null,
    gate,
    delay_min: 0,
    cfr_issued: false,
    category: "M",
    dep: "KJFK",
    aircraft_type: "B738",
  }) as unknown as FlowFlight;

const flowOf = (flights: FlowFlight[], stars = fixture.stars) =>
  ({ icao: fixture.airport, aar: null, flights, stars }) as unknown as Flow;

describe("static and live gate names group the same way (#791)", () => {
  it("has cases to check", () => {
    expect(fixture.cases.length).toBeGreaterThan(0);
  });

  it.each(fixture.cases.map((c) => [c.name, c] as const))("%s: the STAR and the filed gate share a column", (_n, c) => {
    expect(summaryGateName(c.star)).toBe(c.column);
    expect(summaryGateName(c.gate)).toBe(c.column);
  });

  it.each(fixture.cases.map((c) => [c.name, c] as const))(
    "%s: a gate picked from the static list fills when a flight files it",
    (_n, c) => {
      expect(ladderGateOptions(flowOf([]))).toContain(c.column);
      const flow = flowOf([arrival("AAL1", 10, c.gate)]);
      const filters = { gates: [c.column] };
      const columns = airportTguiColumns(airportLadderItems(flow, filters, NOW), NOW, filters.gates);
      expect(columns.map((col) => [col.name, col.items.map((i) => i.key)])).toEqual([[c.column, ["AAL1"]]]);
    },
  );
});

describe("ladderGateOptions (#791)", () => {
  it("lists the airport's STARs, grouped, with no traffic", () => {
    expect(ladderGateOptions(flowOf([]))).toEqual(["CAVLR", "DELRO", "SEG", "WIGOL"]);
  });

  it("merges live gates (a plain-fix gate too) and keeps a saved gate nobody is filed on", () => {
    const flow = flowOf([arrival("AAL1", 10, "CAMRN"), arrival("UAL2", 12, "DELRO5")]);
    expect(ladderGateOptions(flow, ["LENDY"])).toEqual(["CAMRN", "CAVLR", "DELRO", "LENDY", "SEG", "WIGOL"]);
  });
});

describe("airportTguiColumns with a gate filter (#791)", () => {
  const flow = flowOf([arrival("AAL1", 10, "DELRO5"), arrival("X1", 15, null)]);

  it("stays traffic-only when no gate filter names a gate", () => {
    const columns = airportTguiColumns(airportLadderItems(flow, undefined, NOW), NOW);
    expect(columns.map((c) => c.name)).toEqual(["DELRO", "OTHER"]);
  });

  it("gives each filtered gate with no traffic an empty column, sorted, after the gates with traffic", () => {
    const filters = { gates: ["WIGOL", "SEG", "DELRO"] };
    const columns = airportTguiColumns(airportLadderItems(flow, filters, NOW), NOW, filters.gates);
    expect(columns.map((c) => [c.name, c.items.length])).toEqual([
      ["DELRO", 1],
      ["SEG", 0],
      ["WIGOL", 0],
    ]);
  });
});

describe("the TGUI widget's gate filter (#791)", () => {
  const widget = (gates?: string[]): ViewWidget => ({
    id: "w1",
    kind: "view",
    view: "airport-tgui",
    icao: fixture.airport,
    ...(gates ? { filters: { gates } } : {}),
  });

  function render(node: React.ReactNode, flow: Flow, settings: Record<string, unknown> | null = null) {
    const qc = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          refetchInterval: false,
          refetchOnMount: false,
          refetchOnWindowFocus: false,
          refetchOnReconnect: false,
          staleTime: Infinity,
        },
      },
    });
    qc.setQueryData(["preferences", "settings"], settings);
    qc.setQueryData(["flow", fixture.airport], flow);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({ root, host });
    act(() => root.render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>));
    return host;
  }

  function mount(w: ViewWidget, flow: Flow, onChange: (id: string, patch: Record<string, unknown>) => void) {
    const host = render(<ViewWidgetView widget={w} editing onChange={onChange} />, flow);
    const filtersButton = [...host.querySelectorAll("button")].find((b) => b.textContent?.startsWith("Filters"))!;
    act(() => filtersButton.click());
    return host;
  }

  const chip = (host: HTMLElement, name: string) =>
    [...host.querySelectorAll("button")].find((b) => b.textContent === name);

  it("offers the airport's STARs with no live arrivals, and picking one saves it", () => {
    const onChange = vi.fn();
    const host = mount(widget(), flowOf([]), onChange);
    expect(host.textContent).not.toContain("none known");
    for (const g of ["CAVLR", "DELRO", "SEG", "WIGOL"]) expect(chip(host, g)).toBeDefined();
    act(() => chip(host, "WIGOL")!.click());
    expect(onChange).toHaveBeenLastCalledWith("w1", { filters: { gates: ["WIGOL"] } });
  });

  const columnNames = (host: HTMLElement) =>
    [...host.querySelectorAll("[data-column]")].map((c) => c.getAttribute("data-column"));

  it("an unfiltered widget stays traffic-only, with no column per STAR", () => {
    const flow = flowOf([arrival("AAL1", 10, "DELRO5")]);
    const host = render(<ViewWidgetView widget={widget()} editing={false} onChange={() => {}} />, flow);
    expect(columnNames(host)).toEqual(["DELRO"]);
  });

  it("keeps a column for a filtered gate with no traffic", () => {
    const flow = flowOf([arrival("AAL1", 10, "DELRO5")]);
    const host = render(<ViewWidgetView widget={widget(["DELRO", "WIGOL"])} editing={false} onChange={() => {}} />, flow);
    expect(columnNames(host)).toEqual(["DELRO", "WIGOL"]);
  });

  it("the TGUI-style arrival ladder keeps the filtered gate's column too", () => {
    const flow = flowOf([arrival("AAL1", 10, "DELRO5")]);
    const host = render(<LadderView flow={flow} filters={{ gates: ["DELRO", "WIGOL"] }} />, flow, {
      "ladder.style": "tgui",
    });
    expect(columnNames(host)).toEqual(["DELRO", "WIGOL"]);
  });

  it("shows a saved gate with no traffic as a chip that deselects on its own", () => {
    const onChange = vi.fn();
    const host = mount(widget(["LENDY", "WIGOL"]), flowOf([], []), onChange);
    act(() => chip(host, "LENDY")!.click());
    expect(onChange).toHaveBeenLastCalledWith("w1", { filters: { gates: ["WIGOL"] } });
  });
});

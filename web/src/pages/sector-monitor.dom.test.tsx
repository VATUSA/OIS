// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too late
// (VATUSA/OIS#387). Data is seeded into the query cache; the mocked client records every request, a
// GET never answers, and each test says what a PUT answers.
vi.mock("@/lib/api", () => ({ois: {GET: get, PUT: put}, API_BASE: ""}));

import type {Me} from "@/lib/auth";
import {
  type SectorDemand,
  type SectorDemandBin,
  type SectorDemandRow,
  sectorDemandKey,
} from "@/features/sector-demand/sector-demand";
import {SectorMonitorPage} from "./sector-monitor";

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
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
  get.mockReset();
  put.mockReset();
});

/** A scoped reader — never `server_admin`, which would pass every check for the wrong reason. */
function user(permissions: Record<string, unknown>, home = "ZDC"): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: false,
    role_names: [],
    tmu_national: false,
    permissions: permissions as Me["permissions"],
    vatusa: {home_facility: home, roles: [], visits: []},
  } as Me;
}
const READER = user({flow: {sectors: ["read"]}});

const FACILITIES = ["ZDC", "ZLA", "ZNY", "ZOB", "ZSE", "ZID"].map((id) => ({id, name: id, active: true}));

const START = Date.UTC(2026, 9, 7, 4, 15);
const BINS = Array.from({length: 24}, (_, i) => START + i * 15 * 60_000);
const OK: SectorDemandBin = {active: 1, proposed: 0, combined: 1, level: "ok"};

/** A sector green in every bin except `alerts` (bin index → bin). */
function row(id: string, alerts: Record<number, SectorDemandBin> = {}, extra: Partial<SectorDemandRow> = {}): SectorDemandRow {
  return {
    sector_id: id,
    name: null,
    tier: "high",
    limit: 10,
    limit_overridden: false,
    consolidated: [],
    bins: BINS.map((_, i) => alerts[i] ?? OK),
    ...extra,
  };
}
const WATCH: SectorDemandBin = {active: 9, proposed: 3, combined: 12, level: "watch"};
const OVER: SectorDemandBin = {active: 11, proposed: 0, combined: 11, level: "over"};

function demand(artcc: string, patch: Partial<SectorDemand> = {}): SectorDemand {
  return {
    artcc,
    status: "ready",
    cycle_at: new Date(START + 7 * 60_000).toISOString(),
    bin_minutes: 15,
    bin_starts_ms: BINS,
    default_limit: 10,
    limits_editable: false,
    consolidations_editable: false,
    neighbours: [],
    enroute: {has_sector_data: true, rows: [row("16"), row("5", {12: WATCH})]},
    tracon: {has_sector_data: true, rows: [row("80", {0: OVER}, {tier: "approach"})]},
    ...patch,
  };
}

/** ZDC as its TMU sees it: 12 is worked at 10, and TRACON 81 at 80. */
const TMU_BODY = () =>
  demand("ZDC", {
    limits_editable: true,
    consolidations_editable: true,
    enroute: {has_sector_data: true, rows: [row("05"), row("06"), row("10", {}, {consolidated: ["12"]})]},
    tracon: {has_sector_data: true, rows: [row("80", {}, {tier: "approach", consolidated: ["81"]})]},
  });

/**
 * Mounts the page on a cache seeded with `me`, the facilities and each demand body.
 *
 * The alert filter starts on (owner decision, #725), which would hide these quiet fixtures. A test about
 * something else draws every row by remembering the filter as off for each body's tables, the way a
 * viewer who switched it off would; `{defaults: true}` mounts on a clean browser.
 */
async function mount(me: Me | null, bodies: SectorDemand[] = [], {defaults = false}: {defaults?: boolean} = {}) {
  get.mockReturnValue(new Promise(() => {}));
  if (!defaults) {
    for (const b of bodies) {
      for (const kind of ["enroute", "tracon"]) localStorage.setItem(`ois.sectorMonitor.${b.artcc}.${kind}.alertOnly`, "false");
    }
  }
  const qc = new QueryClient({
    defaultOptions: {
      queries: {retry: false, staleTime: Infinity, refetchOnMount: false, refetchOnWindowFocus: false, refetchOnReconnect: false},
      mutations: {retry: false},
    },
  });
  qc.setQueryData(["me"], me);
  qc.setQueryData(["facilities"], FACILITIES);
  for (const b of bodies) qc.setQueryData(sectorDemandKey(b.artcc), b);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <SectorMonitorPage />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc, unmount: () => act(() => root.unmount())};
}

const section = (host: HTMLElement, title: string) => host.querySelector<HTMLElement>(`section[aria-label="${title} sectors"]`);
const names = (el: HTMLElement | null) => [...(el?.querySelectorAll("tbody th[scope=row]") ?? [])].map((th) => th.textContent);
const trOf = (el: HTMLElement | null, sector: string) => el!.querySelector<HTMLElement>(`tr[data-sector="${sector}"]`)!;
const mapCell = (el: HTMLElement | null, sector: string) => el!.querySelector<HTMLElement>(`th[data-map="${sector}"]`)!;
const binCells = (el: HTMLElement | null, sector: string) => [...trOf(el, sector).querySelectorAll<HTMLElement>("td")];
const footer = (el: HTMLElement | null) => [...(el?.querySelectorAll("tfoot th") ?? [])].map((th) => th.textContent);
const demandRequests = () => get.mock.calls.filter(([path]) => path === "/api/v1/flow/sector-demand/{artcc}");
const consolidationPuts = () =>
  put.mock.calls.filter(([path]) => path === "/api/v1/flow/sector-consolidations/{artcc}").map(([, init]) => init);

/** Sets a range input or picks a select option through React's own value tracking. */
const setValue = (el: HTMLInputElement | HTMLSelectElement, value: string) =>
  act(async () => {
    const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
  });
const click = (el: HTMLElement) => act(async () => el.click());
const control = <T extends HTMLElement>(host: HTMLElement, label: string) => host.querySelector<T>(`[aria-label="${label}"]`)!;
const key = (el: HTMLElement, k: string) => act(async () => void el.dispatchEvent(new KeyboardEvent("keydown", {key: k, bubbles: true})));
/** Lets a mutation's promise settle and React render what it did. */
const settle = () => act(async () => void (await new Promise((r) => setTimeout(r, 0))));

const rightClick = (el: HTMLElement) =>
  act(async () => void el.dispatchEvent(new MouseEvent("contextmenu", {bubbles: true, cancelable: true, clientX: 40, clientY: 40})));
const menuRoot = () => document.querySelector<HTMLElement>("[data-sector-menu-root]");
/** A menu row by its exact label. */
const item = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"], [role="menuitemcheckbox"]')].find((el) =>
    [...el.children].some((c) => c.tagName === "SPAN" && c.textContent === label),
  );

const answers = {
  ok: (artcc: string) => ({data: {artcc, editable: true, consolidations: []}, error: undefined, response: {status: 200}}),
  refused: (status: number) => ({data: undefined, error: {code: "refused"}, response: {status}}),
};

describe("the Sector Monitor page (#794)", () => {
  it("is closed without flow.sectors.read, and asks for nothing", async () => {
    const {host} = await mount(user({tmu: {program: ["read"]}}), [demand("ZDC")]);
    expect(host.textContent).toContain("You don't have access to sector demand.");
    expect(host.querySelector("table, section")).toBeNull();
    expect(get).not.toHaveBeenCalled();
  });

  it("draws the home facility's enroute and TRACON tables in vTBFM's layout", async () => {
    const {host} = await mount(READER, [demand("ZDC")]);
    expect(control<HTMLSelectElement>(host, "Facility").value).toBe("ZDC");
    const enroute = section(host, "ZDC");
    const tracon = section(host, "ZDC TRACON");
    // Name cells read ARTCC + sector, in numeric order (5 before 16), with MAP as `10/10`.
    expect(names(enroute)).toEqual(["ZDC5", "ZDC16"]);
    expect(names(tracon)).toEqual(["ZDC80"]);
    expect(mapCell(enroute, "5").textContent).toBe("10/10");
    // A bottom footer, not a header: blank, MAP, then 4 h of HHMM with no Z.
    expect(enroute!.querySelector("thead")).toBeNull();
    const foot = footer(enroute);
    expect(foot.slice(0, 4)).toEqual(["", "MAP", "0415", "0430"]);
    expect(foot).toHaveLength(2 + 16);
    expect(foot.at(-1)).toBe("0800");
    // Each bin shows the combined peak only.
    expect(binCells(enroute, "5")[12].textContent).toBe("12");
    // The controls, as vTBFM labels them.
    expect(enroute!.textContent).toContain("Time Range:");
    expect(enroute!.textContent).toContain("4.00 hours.");
    expect(enroute!.textContent).toContain("Show if alerted in next:");
    expect(enroute!.textContent).toContain("hours (Flow Limit)");
    expect(
      [...control<HTMLSelectElement>(host, "ZDC alert span (hours)").options].map((o) => o.textContent),
    ).toEqual(["1.00", "1.50", "2.00", "2.25", "3.00", "4.00", "5.00", "6.00"]);
  });

  it("colours each bin from the level the server judged, at its strict boundaries", async () => {
    // Limit 10. At the limit is green; combined over it is yellow; active alone over it is red; and a
    // level the server judged is never re-judged here (a 50 the server called ok stays green).
    const bins = {
      0: {active: 10, proposed: 0, combined: 10, level: "ok"},
      1: {active: 10, proposed: 1, combined: 11, level: "watch"},
      2: {active: 11, proposed: 0, combined: 11, level: "over"},
      3: {active: 50, proposed: 0, combined: 50, level: "ok"},
    } satisfies Record<number, SectorDemandBin>;
    const {host} = await mount(READER, [demand("ZDC", {enroute: {has_sector_data: true, rows: [row("25", bins)]}})]);
    const cells = binCells(section(host, "ZDC"), "25");
    expect(cells.slice(0, 4).map((c) => c.dataset.colour)).toEqual(["green", "yellow", "red", "green"]);
    expect(cells.slice(0, 4).map((c) => c.style.background)).toEqual([
      "rgb(61, 199, 68)",
      "rgb(242, 247, 42)",
      "rgb(217, 15, 16)",
      "rgb(61, 199, 68)",
    ]);
    expect(cells[2].title).toBe("ZDC25 0445Z · peak 11 (airborne 11) vs MAP 10 · red");
  });

  it("re-slices the range in whole hours with no refetch", async () => {
    const {host} = await mount(READER, [demand("ZDC")]);
    const range = control<HTMLInputElement>(host, "ZDC time range (hours)");
    expect([range.min, range.max, range.step, range.value]).toEqual(["2", "6", "1", "4"]);
    await setValue(range, "2");
    expect(footer(section(host, "ZDC"))).toHaveLength(2 + 8);
    expect(section(host, "ZDC")!.textContent).toContain("2.00 hours.");
    await setValue(range, "6");
    expect(footer(section(host, "ZDC"))).toHaveLength(2 + 24);
    // The TRACON table keeps its own range.
    expect(footer(section(host, "ZDC TRACON"))).toHaveLength(2 + 16);
    expect(demandRequests()).toHaveLength(0);
  });

  it("opens filtered to sectors alerting in the next 2 hours, on a span independent of the range", async () => {
    // 5 alerts only at bin 12 (3 h out). On a clean browser the filter is on at 2.00 h, so only it is gone.
    const body = demand("ZDC", {enroute: {has_sector_data: true, rows: [row("16", {7: WATCH}), row("5", {12: WATCH})]}});
    const {host} = await mount(READER, [body], {defaults: true});
    const enroute = section(host, "ZDC");
    expect(control<HTMLInputElement>(host, "ZDC: show if alerted").checked).toBe(true);
    expect(control<HTMLSelectElement>(host, "ZDC alert span (hours)").value).toBe("8");
    expect(names(enroute)).toEqual(["ZDC16"]);
    // Draw 2 h, filter on 4 h: 5 comes back although its alert is outside the drawn range.
    await setValue(control(host, "ZDC time range (hours)"), "2");
    await setValue(control(host, "ZDC alert span (hours)"), "16");
    expect(names(enroute)).toEqual(["ZDC5", "ZDC16"]);
    expect(footer(enroute)).toHaveLength(2 + 8);
    // And the range never moves the filter: back to 6 h, still the same two rows.
    await setValue(control(host, "ZDC time range (hours)"), "6");
    expect(names(enroute)).toEqual(["ZDC5", "ZDC16"]);
    await click(control(host, "ZDC: show if alerted"));
    expect(names(enroute)).toEqual(["ZDC5", "ZDC16"]);
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.alertOnly")).toBe("false");
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.alertSpan")).toBe("16");
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.range")).toBe("6");
  });

  describe("states, each said in words rather than drawn as an empty grid", () => {
    const text = (host: HTMLElement, title: string) => section(host, title)!.textContent;

    it("waits for the first cycle, both before the read and while the server is pending", async () => {
      const loading = await mount(READER, []);
      expect(text(loading.host, "ZDC")).toContain("Waiting for the first sector-monitor cycle…");
      await loading.unmount();
      const {host} = await mount(READER, [demand("ZDC", {status: "pending", enroute: {has_sector_data: true, rows: []}})]);
      for (const t of ["ZDC", "ZDC TRACON"]) {
        expect(text(host, t)).toContain("Waiting for the first sector-monitor cycle…");
        expect(section(host, t)!.querySelector("table")).toBeNull();
      }
    });

    it("names a facility with no sector data in one table", async () => {
      const {host} = await mount(user({flow: {sectors: ["read"]}}, "ZLA"), [
        demand("ZLA", {status: "no_sector_data", enroute: {has_sector_data: false, rows: []}, tracon: {has_sector_data: false, rows: []}}),
      ]);
      expect(text(host, "ZLA")).toContain("No sector data for ZLA");
      expect(section(host, "ZLA TRACON")).toBeNull();
      expect(host.querySelector("table")).toBeNull();
    });

    it("names a table the dataset has no volumes for, while drawing the other", async () => {
      const {host} = await mount(user({flow: {sectors: ["read"]}}, "ZSE"), [
        demand("ZSE", {tracon: {has_sector_data: false, rows: []}}),
      ]);
      expect(text(host, "ZSE TRACON")).toContain("No TRACON sector data for ZSE");
      expect(names(section(host, "ZSE"))).toEqual(["ZSE5", "ZSE16"]);
    });

    it("says when a table has no sectors, and when the filter hides every one", async () => {
      const empty = await mount(READER, [demand("ZDC", {enroute: {has_sector_data: true, rows: []}, tracon: {has_sector_data: true, rows: []}})]);
      expect(text(empty.host, "ZDC")).toContain("No sectors for ZDC.");
      expect(text(empty.host, "ZDC TRACON")).toContain("No TRACON sectors for ZDC.");
      await empty.unmount();
      localStorage.clear();
      const quiet = demand("ZDC", {enroute: {has_sector_data: true, rows: [row("16")]}, tracon: {has_sector_data: true, rows: [row("80")]}});
      const {host} = await mount(READER, [quiet], {defaults: true});
      expect(text(host, "ZDC")).toContain("No ZDC sectors alerting in the next 2.00 h.");
      expect(text(host, "ZDC TRACON")).toContain("No ZDC TRACON sectors alerting in the next 2.00 h.");
    });
  });

  it("remembers each table's controls per ARTCC and table, and works when storage throws", async () => {
    put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
    const bodies = () => [{...TMU_BODY(), neighbours: ["ZNY"]}, demand("ZNY")];
    const first = await mount(READER, bodies());
    await setValue(control(first.host, "ZDC time range (hours)"), "3");
    await setValue(control(first.host, "ZDC alert span (hours)"), "12");
    await rightClick(trOf(section(first.host, "ZDC"), "05"));
    await click(item("Move Row Down")!);
    await click(control(first.host, "Expand ZNY TRACON"));
    await click(control(first.host, "Collapse ZDC controls"));
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.range")).toBe("3");
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.collapsed")).toBe("true");
    expect(localStorage.getItem("ois.sectorMonitor.ZDC.tracon.range")).toBeNull();
    await first.unmount();

    const again = await mount(READER, bodies());
    // Collapsed: the controls are folded away, the grid stays, in this browser's order.
    expect(again.host.querySelector('[aria-label="ZDC time range (hours)"]')).toBeNull();
    expect(names(section(again.host, "ZDC"))).toEqual(["ZDC06", "ZDC05", "ZDC10+"]);
    await click(control(again.host, "Expand ZDC controls"));
    expect(control<HTMLInputElement>(again.host, "ZDC time range (hours)").value).toBe("3");
    expect(control<HTMLSelectElement>(again.host, "ZDC alert span (hours)").value).toBe("12");
    expect(control<HTMLInputElement>(again.host, "ZDC TRACON time range (hours)").value).toBe("4");
    // The neighbour's TRACON table was left open, its enroute table closed.
    expect(control(again.host, "Collapse ZNY TRACON").getAttribute("aria-expanded")).toBe("true");
    expect(control(again.host, "Expand ZNY").getAttribute("aria-expanded")).toBe("false");
    expect(Object.keys(localStorage).every((k) => /^ois\.sectorMonitor\.(ZDC|ZNY)\.(enroute|tracon)\./.test(k))).toBe(true);
    await again.unmount();

    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const blocked = await mount(READER, [demand("ZDC")], {defaults: true});
    await setValue(control(blocked.host, "ZDC time range (hours)"), "5");
    expect(control<HTMLInputElement>(blocked.host, "ZDC time range (hours)").value).toBe("5");
  });

  it("collapses neighbours, fetches one only when opened, and never offers them a MAP editor or menu", async () => {
    // A national TMU: the neighbour's own body says it is editable, and the page still says no.
    const zny = demand("ZNY", {limits_editable: true, consolidations_editable: true});
    // ZNY is not in the cache, so any enabled query for it would fetch on mount.
    localStorage.setItem("ois.sectorMonitor.ZNY.enroute.alertOnly", "false");
    const {host, qc} = await mount(READER, [demand("ZDC", {neighbours: ["ZNY"], limits_editable: true, consolidations_editable: true})]);
    for (const t of ["ZNY", "ZNY TRACON"]) {
      expect(control(host, `Expand ${t}`).getAttribute("aria-expanded")).toBe("false");
      expect(section(host, t)!.querySelector("table, input")).toBeNull();
    }
    await settle();
    expect(demandRequests()).toHaveLength(0);
    await click(control(host, "Expand ZNY"));
    expect(demandRequests().map(([, init]) => init.params.path.artcc)).toEqual(["ZNY"]);
    expect(section(host, "ZNY")!.textContent).toContain("Waiting for the first sector-monitor cycle…");
    await act(async () => {
      qc.setQueryData(sectorDemandKey("ZNY"), zny);
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(localStorage.getItem("ois.sectorMonitor.ZNY.enroute.open")).toBe("true");
    const table = section(host, "ZNY");
    expect(names(table)).toEqual(["ZNY5", "ZNY16"]);
    expect(section(host, "ZNY TRACON")!.querySelector("table")).toBeNull();
    // No MAP editor: no hint, no input on click. No menu: a right-click opens nothing.
    expect(mapCell(table, "5").title).toBe("");
    await click(mapCell(table, "5"));
    expect(table!.querySelector("input[type=number]")).toBeNull();
    await rightClick(trOf(table, "5"));
    expect(menuRoot()).toBeNull();
    // Control: the facility's own table does both.
    await rightClick(trOf(section(host, "ZDC"), "5"));
    expect(menuRoot()).not.toBeNull();
  });

  it("replaces every table on a facility switch, each starting from its own state", async () => {
    const {host} = await mount(READER, [demand("ZDC", {neighbours: ["ZNY"]}), demand("ZOB", {neighbours: ["ZID"]}), demand("ZNY")]);
    // Storage swallowed, so only live state could carry across.
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {});
    await setValue(control(host, "ZDC time range (hours)"), "2");
    await setValue(control(host, "ZDC alert span (hours)"), "12");
    await click(control(host, "Expand ZNY"));
    expect(section(host, "ZNY")).not.toBeNull();
    await setValue(control(host, "Facility"), "ZOB");
    expect(host.querySelector('section[aria-label^="ZDC"], section[aria-label^="ZNY"]')).toBeNull();
    expect(names(section(host, "ZOB"))).toEqual(["ZOB5", "ZOB16"]);
    expect(control<HTMLInputElement>(host, "ZOB time range (hours)").value).toBe("4");
    expect(control<HTMLSelectElement>(host, "ZOB alert span (hours)").value).toBe("8");
    expect(control(host, "Expand ZID").getAttribute("aria-expanded")).toBe("false");
  });

  it("never remembers the facility pick: a reload opens on the viewer's home facility", async () => {
    const bodies = [demand("ZDC"), demand("ZOB"), demand("ZLA")];
    const first = await mount(READER, bodies);
    await setValue(control(first.host, "Facility"), "ZOB");
    expect(section(first.host, "ZOB")).not.toBeNull();
    await first.unmount();

    const again = await mount(READER, bodies);
    expect(control<HTMLSelectElement>(again.host, "Facility").value).toBe("ZDC");
    expect(again.host.querySelector('section[aria-label^="ZOB"]')).toBeNull();
    await again.unmount();

    // A controller who moved from ZDC to ZLA opens on ZLA, with no ZDC table left over.
    const moved = await mount(user({flow: {sectors: ["read"]}}, "ZLA"), bodies);
    expect(control<HTMLSelectElement>(moved.host, "Facility").value).toBe("ZLA");
    expect(moved.host.querySelector('section[aria-label^="ZDC"]')).toBeNull();
    expect(Object.keys(localStorage).filter((k) => !k.startsWith("ois.sectorMonitor."))).toEqual([]);
  });

  describe("MAP inline edit (#794, #722's PUT)", () => {
    const tmu = () => mount(READER, [TMU_BODY()]);

    it("offers no editor where the server says the viewer may not set limits", async () => {
      const {host} = await mount(READER, [demand("ZDC")]);
      const enroute = section(host, "ZDC");
      expect(mapCell(enroute, "5").title).toBe("");
      await click(mapCell(enroute, "5"));
      expect(enroute!.querySelector("input[type=number]")).toBeNull();
    });

    it("commits a changed positive number on Enter, shows it at once, and refetches", async () => {
      put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
      const {host} = await tmu();
      const enroute = section(host, "ZDC");
      expect(mapCell(enroute, "05").title).toBe("Click to edit MAP");
      await click(mapCell(enroute, "05"));
      const input = control<HTMLInputElement>(host, "MAP for ZDC05");
      expect([input.min, input.style.width]).toEqual(["1", "34px"]);
      await setValue(input, "12");
      await key(input, "Enter");
      expect(put).toHaveBeenCalledWith("/api/v1/flow/sector-limits/{artcc}/{sector_id}", {
        params: {path: {artcc: "ZDC", sector_id: "05"}},
        body: {limit: 12},
      });
      expect(mapCell(enroute, "05").textContent).toBe("12/12");
      await settle();
      expect(demandRequests().length).toBeGreaterThan(0);
      // The far side of the boundary: 1 is a positive number, and commits.
      await click(mapCell(enroute, "06"));
      await setValue(control(host, "MAP for ZDC06"), "1");
      await key(control(host, "MAP for ZDC06"), "Enter");
      expect(put.mock.calls.map(([, init]) => init.body)).toEqual([{limit: 12}, {limit: 1}]);
    });

    it("hands the cell back to the server's answer once the refetch lands", async () => {
      put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
      const {host} = await tmu();
      const served = TMU_BODY();
      served.enroute.rows[0] = {...served.enroute.rows[0], limit: 11};
      get.mockResolvedValue({data: served, error: undefined});
      const enroute = section(host, "ZDC");
      await click(mapCell(enroute, "05"));
      await setValue(control(host, "MAP for ZDC05"), "12");
      await key(control(host, "MAP for ZDC05"), "Enter");
      await settle();
      await settle();
      expect(mapCell(enroute, "05").textContent).toBe("11/11");
    });

    it("cancels on Escape, and on blur with nothing changed, zero or a negative", async () => {
      const {host} = await tmu();
      const enroute = section(host, "ZDC");
      for (const [draft, end] of [["14", "Escape"], ["10", "blur"], ["0", "Enter"], ["-3", "blur"], ["", "Enter"]] as const) {
        await click(mapCell(enroute, "05"));
        const input = control<HTMLInputElement>(host, "MAP for ZDC05");
        await setValue(input, draft);
        if (end === "blur") await act(async () => input.blur());
        else await key(input, end);
        expect(enroute!.querySelector("input[type=number]"), `${draft} ${end}`).toBeNull();
      }
      expect(put).not.toHaveBeenCalled();
      expect(mapCell(enroute, "05").textContent).toBe("10/10");
    });

    it("commits on blur too", async () => {
      put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
      const {host} = await tmu();
      await click(mapCell(section(host, "ZDC"), "06"));
      const input = control<HTMLInputElement>(host, "MAP for ZDC06");
      await setValue(input, "7");
      await act(async () => input.blur());
      expect(put.mock.calls[0][1].body).toEqual({limit: 7});
    });

    it("rolls back the first of two overlapping edits on its own refusal", async () => {
      let answerFirst: (v: unknown) => void = () => {};
      put.mockImplementationOnce(() => new Promise((r) => (answerFirst = r))).mockResolvedValueOnce({data: {}, error: undefined, response: {status: 200}});
      const {host} = await tmu();
      const enroute = section(host, "ZDC");
      for (const [sector, value] of [["05", "3"], ["06", "4"]]) {
        await click(mapCell(enroute, sector));
        await setValue(control(host, `MAP for ZDC${sector}`), value);
        await key(control(host, `MAP for ZDC${sector}`), "Enter");
      }
      expect([mapCell(enroute, "05").textContent, mapCell(enroute, "06").textContent]).toEqual(["03/03", "04/04"]);
      await act(async () => answerFirst(answers.refused(403)));
      await settle();
      // 05 is back on the stored value; 06's write, which succeeded, still shows until its refetch lands.
      expect([mapCell(enroute, "05").textContent, mapCell(enroute, "06").textContent]).toEqual(["10/10", "04/04"]);
      expect(host.querySelector('[role="alert"]')!.textContent).toBe("Could not save MAP for ZDC05 — check TMU access / connection.");
    });

    it("rolls a refused write back and says so", async () => {
      put.mockResolvedValue(answers.refused(403));
      const {host} = await tmu();
      const enroute = section(host, "ZDC");
      await click(mapCell(enroute, "05"));
      await setValue(control(host, "MAP for ZDC05"), "3");
      await key(control(host, "MAP for ZDC05"), "Enter");
      await settle();
      expect(mapCell(enroute, "05").textContent).toBe("10/10");
      expect(host.querySelector('[role="alert"]')!.textContent).toBe(
        "Could not save MAP for ZDC05 — check TMU access / connection.",
      );
      expect(demandRequests()).toHaveLength(0);
    });
  });

  describe("the right-click menu: the consolidation editor (#794, #792)", () => {
    /** Opens the menu on `sector`'s row of ZDC's enroute table. */
    async function menuOn(host: HTMLElement, sector: string) {
      await rightClick(trOf(section(host, "ZDC"), sector));
      expect(menuRoot()).not.toBeNull();
    }
    const run = async (...labels: string[]) => {
      for (const l of labels) await click(item(l)!);
    };

    it("does not open without the right to change consolidations, even for a viewer who sets limits", async () => {
      const {host} = await mount(READER, [{...TMU_BODY(), consolidations_editable: false}]);
      const tr = trOf(section(host, "ZDC"), "05");
      expect(tr.querySelector("th")!.title).toBe("");
      await rightClick(tr);
      expect(menuRoot()).toBeNull();
    });

    it("lists vTBFM's commands, named for the target", async () => {
      const {host} = await mount(READER, [TMU_BODY()]);
      expect(trOf(section(host, "ZDC"), "06").querySelector("th")!.title).toBe("Right-click for row and consolidation commands");
      // A target others are worked at reads with a trailing +.
      expect(names(section(host, "ZDC"))).toEqual(["ZDC05", "ZDC06", "ZDC10+"]);
      await menuOn(host, "06");
      expect(menuRoot()!.parentElement).toBe(document.body);
      await run("Consolidate");
      expect(item("Consolidate All into 06")).toBeDefined();
      expect(item("Consolidate All into 06 Except Consolidated")).toBeDefined();
      await run("Consolidate into 06");
      // Every other sector not already worked elsewhere: 12 is worked at 10, so only 10 is offered.
      const list = [...document.querySelectorAll('[role="menuitemcheckbox"]')].map((el) => el.textContent);
      expect(list).toEqual(["05", "10"]);
      await run("Deconsolidate");
      expect(item("Deconsolidate All from 06")!.getAttribute("aria-disabled")).toBe("true");
      expect(item("Deconsolidate All in ZDC")!.getAttribute("aria-disabled")).toBeNull();
    });

    it("moves rows up and down, remembered per browser", async () => {
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "05");
      expect(item("Move Row Up")!.getAttribute("aria-disabled")).toBe("true");
      await run("Move Row Down");
      expect(menuRoot()).toBeNull();
      expect(names(section(host, "ZDC"))).toEqual(["ZDC06", "ZDC05", "ZDC10+"]);
      await menuOn(host, "10");
      expect(item("Move Row Down")!.getAttribute("aria-disabled")).toBe("true");
      await run("Move Row Up");
      expect(names(section(host, "ZDC"))).toEqual(["ZDC06", "ZDC10+", "ZDC05"]);
      expect(JSON.parse(localStorage.getItem("ois.sectorMonitor.ZDC.enroute.order")!)).toEqual(["06", "10", "05"]);
      expect(put).not.toHaveBeenCalled();
    });

    it("sends Consolidate All into T as one batch, shows it at once and refetches", async () => {
      put.mockResolvedValue(answers.ok("ZDC"));
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate All into 06");
      expect(consolidationPuts()).toEqual([{params: {path: {artcc: "ZDC"}}, body: {into: {"05": "06", "10": "06", "12": "06"}}}]);
      expect(menuRoot()).toBeNull();
      // Optimistic: 05 and 10 leave the board and 06 reads as a target, before any refetch lands.
      expect(names(section(host, "ZDC"))).toEqual(["ZDC06+"]);
      await settle();
      expect(demandRequests().length).toBeGreaterThan(0);
    });

    it("hands the board back to the server's answer once the refetch lands", async () => {
      put.mockResolvedValue(answers.ok("ZDC"));
      const {host} = await mount(READER, [TMU_BODY()]);
      // Another TMU's change landed first: the server answers with 05 on its own row.
      get.mockResolvedValue({data: TMU_BODY(), error: undefined});
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate into 06", "05");
      await settle();
      await settle();
      expect(names(section(host, "ZDC"))).toEqual(["ZDC05", "ZDC06", "ZDC10+"]);
    });

    it("rolls back the first of two overlapping writes on its own refusal", async () => {
      let answerFirst: (v: unknown) => void = () => {};
      put.mockImplementationOnce(() => new Promise((r) => (answerFirst = r))).mockResolvedValueOnce(answers.ok("ZDC"));
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate into 06", "05", "10");
      expect(consolidationPuts().map((c) => c.body.into)).toEqual([{"05": "06"}, {"10": "06"}]);
      expect(names(section(host, "ZDC"))).toEqual(["ZDC06+"]);
      await act(async () => answerFirst(answers.refused(409)));
      await settle();
      // 05 is back; 10's write, which succeeded, still shows until its refetch lands.
      expect(names(section(host, "ZDC"))).toEqual(["ZDC05", "ZDC06+"]);
      expect(host.querySelector('[role="alert"]')!.textContent).toBe("Can't consolidate ZDC05 into ZDC06: ZDC06 is worked at ZDC05.");
    });

    it("disables Deconsolidate when the ARTCC has nothing combined", async () => {
      const body = TMU_BODY();
      body.enroute.rows[2] = {...body.enroute.rows[2], consolidated: []};
      body.tracon.rows[0] = {...body.tracon.rows[0], consolidated: []};
      const {host} = await mount(READER, [body]);
      vi.useFakeTimers();
      try {
        await menuOn(host, "06");
        expect(item("Deconsolidate")!.getAttribute("aria-disabled")).toBe("true");
        await run("Deconsolidate");
        await act(async () => void item("Deconsolidate")!.dispatchEvent(new MouseEvent("mouseover", {bubbles: true})));
        await act(async () => void vi.advanceTimersByTime(500));
        expect(item("Deconsolidate All in ZDC")).toBeUndefined();
      } finally {
        vi.useRealTimers();
      }
    });

    it("leaves every sector in an arrangement alone for Except Consolidated", async () => {
      put.mockResolvedValue(answers.ok("ZDC"));
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate All into 06 Except Consolidated");
      expect(consolidationPuts()).toEqual([{params: {path: {artcc: "ZDC"}}, body: {into: {"05": "06"}}}]);
    });

    it("toggles single sectors from the checklist and stays open", async () => {
      put.mockResolvedValue(answers.ok("ZDC"));
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate into 06", "05");
      expect(menuRoot()).not.toBeNull();
      expect(item("05")!.getAttribute("aria-checked")).toBe("true");
      await run("05");
      expect(consolidationPuts().map((c) => c.body.into)).toEqual([{"05": "06"}, {"05": null}]);
      expect(item("05")!.getAttribute("aria-checked")).toBe("false");
    });

    for (const [command, into] of [
      [["Deconsolidate All from 10"], {"12": null}],
      // ARTCC-wide: the TRACON's 81 too.
      [["Deconsolidate All in ZDC"], {"12": null, "81": null}],
      [["Deconsolidate from 10", "12"], {"12": null}],
    ] as const) {
      it(`sends ${command.join(" ▸ ")} as one batch`, async () => {
        put.mockResolvedValue(answers.ok("ZDC"));
        const {host} = await mount(READER, [TMU_BODY()]);
        await menuOn(host, "10");
        await run("Deconsolidate", ...command);
        expect(consolidationPuts().map((c) => c.body.into)).toEqual([into]);
        // The checklist stays open; every other command closes the menu.
        expect(menuRoot() !== null).toBe(command.length === 2);
        // Optimistic: 12 has its own row back, ahead of the refetch (12 has no row in this fixture's
        // data, so the + is what goes).
        expect(names(section(host, "ZDC"))).toEqual(["ZDC05", "ZDC06", "ZDC10"]);
      });
    }

    it("closes one level per Escape and all of it on a click outside", async () => {
      const {host} = await mount(READER, [TMU_BODY()]);
      await menuOn(host, "06");
      await run("Consolidate", "Consolidate into 06");
      expect(item("05")).toBeDefined();
      await key(document.body, "Escape");
      expect(item("05")).toBeUndefined();
      expect(item("Consolidate All into 06")).toBeDefined();
      await key(document.body, "Escape");
      expect(item("Consolidate All into 06")).toBeUndefined();
      expect(menuRoot()).not.toBeNull();
      await key(document.body, "Escape");
      expect(menuRoot()).toBeNull();

      await menuOn(host, "06");
      await act(async () => void item("Consolidate")!.dispatchEvent(new MouseEvent("mousedown", {bubbles: true})));
      expect(menuRoot()).not.toBeNull();
      await act(async () => void host.dispatchEvent(new MouseEvent("mousedown", {bubbles: true})));
      expect(menuRoot()).toBeNull();
    });

    it("opens a submenu after a 180 ms hover", async () => {
      vi.useFakeTimers();
      try {
        const {host} = await mount(READER, [TMU_BODY()]);
        await menuOn(host, "06");
        await act(async () => void item("Consolidate")!.dispatchEvent(new MouseEvent("mouseover", {bubbles: true})));
        await act(async () => void vi.advanceTimersByTime(179));
        expect(item("Consolidate All into 06")).toBeUndefined();
        await act(async () => void vi.advanceTimersByTime(1));
        expect(item("Consolidate All into 06")).toBeDefined();
      } finally {
        vi.useRealTimers();
      }
    });

    for (const [status, text] of [
      [400, "That change names a sector twice, or too many; nothing was saved."],
      [403, "You can't change ZDC's consolidations."],
      [404, "A sector in that change is no longer one of ZDC's sectors; nothing was saved."],
      [409, "Can't consolidate ZDC05 into ZDC06: ZDC06 is worked at ZDC05."],
      [500, "Could not save the consolidation — check TMU access / connection."],
    ] as const) {
      it(`rolls a ${status} back and says why in words`, async () => {
        put.mockResolvedValue(answers.refused(status));
        const {host} = await mount(READER, [TMU_BODY()]);
        await menuOn(host, "06");
        await run("Consolidate", "Consolidate into 06", "05");
        await settle();
        expect(host.querySelector('[role="alert"]')!.textContent).toBe(text);
        expect(names(section(host, "ZDC"))).toEqual(["ZDC05", "ZDC06", "ZDC10+"]);
        expect(demandRequests()).toHaveLength(0);
      });
    }
  });
});

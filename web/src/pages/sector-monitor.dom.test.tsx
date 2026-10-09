// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too late
// (VATUSA/OIS#387). Data is seeded into the query cache; the mocked client only records what a test
// did not seed, and never answers.
vi.mock("@/lib/api", () => ({ois: {GET: get, PUT: put}, API_BASE: ""}));

import type {Me} from "@/lib/auth";
import {
  type SectorDemand,
  type SectorDemandBin,
  type SectorDemandRow,
  sectorDemandKey,
} from "@/features/sector-demand/sector-demand";
import {DEFAULT_VIEW, saveView} from "@/features/sector-demand/view";
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

const FACILITIES = ["ZDC", "ZLA", "ZNY", "ZOB", "ZSE"].map((id) => ({id, name: id, active: true}));

const START = Date.UTC(2026, 9, 7, 14, 0);
const BINS = Array.from({length: 24}, (_, i) => START + i * 15 * 60_000);
const OK: SectorDemandBin = {active: 1, proposed: 0, combined: 1, level: "ok"};

/** A sector green in every bin except `alerts` (bin index → level). */
function row(id: string, alerts: Record<number, SectorDemandBin["level"]> = {}, extra: Partial<SectorDemandRow> = {}): SectorDemandRow {
  return {
    sector_id: id,
    tier: "high",
    limit: 10,
    limit_overridden: false,
    consolidated: [],
    bins: BINS.map((_, i) => (alerts[i] ? {active: 9, proposed: 3, combined: 12, level: alerts[i]} : OK)),
    ...extra,
  };
}

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
    enroute: {has_sector_data: true, rows: [row(`${artcc}05`), row(`${artcc}06`, {12: "watch"})]},
    tracon: {has_sector_data: true, rows: [row(`${artcc}80`, {0: "over"}, {tier: "approach"})]},
    ...patch,
  };
}

/**
 * Mounts the page on a cache seeded with `me`, the facilities and each demand body.
 *
 * The alert filter starts on (owner decision, #725), which would hide most of these fixtures' quiet
 * rows. A test about something else draws every row by remembering the filter as off for each body's
 * facility, the way a viewer who switched it off would; `{defaults: true}` mounts on a clean browser.
 */
async function mount(me: Me | null, bodies: SectorDemand[] = [], {defaults = false}: {defaults?: boolean} = {}) {
  get.mockReturnValue(new Promise(() => {}));
  if (!defaults) {
    for (const b of bodies) {
      for (const kind of ["enroute", "tracon"] as const) saveView(b.artcc, kind, {...DEFAULT_VIEW, alertOnly: false});
    }
  }
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
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

const section = (host: HTMLElement, name: string) => host.querySelector<HTMLElement>(`section[aria-label="${name}"]`);
const sectorIds = (el: HTMLElement | null) =>
  [...(el?.querySelectorAll("tbody th") ?? [])].map((th) => th.firstChild?.textContent);
const levelCells = (el: HTMLElement | null) => el?.querySelectorAll("[data-level]") ?? [];
const demandRequests = () => get.mock.calls.filter(([path]) => path === "/api/v1/flow/sector-demand/{artcc}");

/** Sets a range input or picks a select option through React's own value tracking. */
const setValue = (el: HTMLInputElement | HTMLSelectElement, value: string) =>
  act(async () => {
    const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
  });
const click = (el: HTMLElement) => act(async () => el.click());
const control = <T extends HTMLElement>(host: HTMLElement, label: string) =>
  host.querySelector<T>(`[aria-label="${label}"]`)!;

describe("the Sector Monitor page (#725)", () => {
  it("is closed without flow.sectors.read, and asks for nothing", async () => {
    const {host} = await mount(user({tmu: {program: ["read"]}}), [demand("ZDC")]);
    expect(host.textContent).toContain("You don't have access to sector demand.");
    expect(host.querySelector("table, select")).toBeNull();
    expect(get).not.toHaveBeenCalled();

    const signedOut = await mount(null, [demand("ZDC")]);
    expect(signedOut.host.querySelector("table")).toBeNull();
  });

  it("opens on the reader's home facility with separate enroute and TRACON tables", async () => {
    const {host} = await mount(READER, [demand("ZDC")]);
    expect(control<HTMLSelectElement>(host, "Facility").value).toBe("ZDC");
    expect(sectorIds(section(host, "ZDC Enroute sectors"))).toEqual(["ZDC05", "ZDC06"]);
    expect(sectorIds(section(host, "ZDC TRACON sectors"))).toEqual(["ZDC80"]);
    // Four hours by default: 16 quarter-hours, the footer in HHMM Zulu from the first.
    expect(levelCells(section(host, "ZDC Enroute sectors"))).toHaveLength(2 * 16);
    const footer = [...section(host, "ZDC Enroute sectors")!.querySelectorAll("tfoot td")].map((c) => c.textContent);
    expect(footer.filter(Boolean).slice(0, 2)).toEqual(["1400", "1415"]);
    expect(host.textContent).toContain("1407z feed cycle");
  });

  it("names a facility with no sector data, and draws no grid", async () => {
    const {host} = await mount(READER, [
      demand("ZDC"),
      demand("ZLA", {status: "no_sector_data", cycle_at: null, bin_starts_ms: [], enroute: {has_sector_data: false, rows: []}, tracon: {has_sector_data: false, rows: []}}),
    ]);
    await setValue(control(host, "Facility"), "ZLA");
    expect(host.textContent).toContain("No sector data for ZLA");
    expect(host.querySelector("table")).toBeNull();
    expect(host.textContent).not.toContain("Waiting for the first cycle");
  });

  it("says it is waiting for the first cycle rather than drawing an empty grid", async () => {
    const {host} = await mount(READER, [
      demand("ZDC", {status: "pending", cycle_at: null, bin_starts_ms: [], enroute: {has_sector_data: true, rows: []}, tracon: {has_sector_data: true, rows: []}}),
    ]);
    expect(host.textContent).toContain("Waiting for the first cycle");
    expect(host.textContent).toContain("ZDC's sector demand");
    // #725 Q5: pending also means the sector data hasn't loaded yet, so the copy names both waits.
    expect(host.textContent).toContain("loaded the sector data");
    expect(host.textContent).toContain("received a feed cycle");
    expect(host.querySelector("table")).toBeNull();
    expect(host.textContent).not.toContain("No sector data");
  });

  it("names a table the dataset has no volumes for (ZSE's TRACON) while drawing the other", async () => {
    const {host} = await mount(READER, [demand("ZSE", {tracon: {has_sector_data: false, rows: []}})]);
    await setValue(control(host, "Facility"), "ZSE");
    expect(section(host, "ZSE TRACON sectors")!.textContent).toContain("No TRACON sector data for ZSE");
    expect(section(host, "ZSE TRACON sectors")!.querySelector("table")).toBeNull();
    expect(sectorIds(section(host, "ZSE Enroute sectors"))).toEqual(["ZSE05", "ZSE06"]);
  });

  it("re-slices the range with no refetch", async () => {
    const {host, qc} = await mount(READER, [demand("ZDC")]);
    const before = qc.getQueryState(sectorDemandKey("ZDC"))!.dataUpdatedAt;
    const enroute = section(host, "ZDC Enroute sectors");
    await setValue(control(host, "ZDC Enroute range"), "2");
    expect(levelCells(enroute)).toHaveLength(2 * 8);
    expect(enroute!.textContent).toContain("2.00 h");
    await setValue(control(host, "ZDC Enroute range"), "6");
    expect(levelCells(enroute)).toHaveLength(2 * 24);
    expect(demandRequests()).toEqual([]);
    expect(qc.getQueryState(sectorDemandKey("ZDC"))!.dataUpdatedAt).toBe(before);
  });

  it("filters on a span independent of the drawn range", async () => {
    const {host} = await mount(READER, [demand("ZDC")]);
    const enroute = section(host, "ZDC Enroute sectors");
    // ZDC06 alerts at 1700Z, three hours out. Draw two hours, filter on the next four.
    await setValue(control(host, "ZDC Enroute range"), "2");
    await setValue(control(host, "ZDC Enroute alert span"), "4");
    await click(control(host, "ZDC Enroute: only sectors alerting"));
    expect(sectorIds(enroute)).toEqual(["ZDC06"]);
    expect(levelCells(enroute)).toHaveLength(8);
    // The same two-hour table filtered on the next 2.75 h hides it — nothing is alerting that soon.
    await setValue(control(host, "ZDC Enroute alert span"), "2.75");
    expect(enroute!.querySelector("table")).toBeNull();
    expect(enroute!.textContent).toContain("No ZDC sectors alerting in the next 2.75 h");
    // Widening the range does not bring it back: the span alone decides.
    await setValue(control(host, "ZDC Enroute range"), "6");
    expect(enroute!.textContent).toContain("No ZDC sectors alerting in the next 2.75 h");
    // The TRACON table has its own controls, untouched.
    expect(sectorIds(section(host, "ZDC TRACON sectors"))).toEqual(["ZDC80"]);
  });

  it("remembers each table's controls per browser per facility", async () => {
    const first = await mount(READER, [demand("ZDC"), demand("ZNY")], {defaults: true});
    await setValue(control(first.host, "ZDC Enroute range"), "3");
    await setValue(control(first.host, "ZDC Enroute alert span"), "1.5");
    await click(control(first.host, "ZDC Enroute: only sectors alerting"));
    await first.unmount();

    const again = await mount(READER, [demand("ZDC"), demand("ZNY")], {defaults: true});
    expect(control<HTMLInputElement>(again.host, "ZDC Enroute range").value).toBe("3");
    expect(control<HTMLSelectElement>(again.host, "ZDC Enroute alert span").value).toBe("1.5");
    // Switched off, and remembered off: the default does not come back on a reload.
    expect(control(again.host, "ZDC Enroute: only sectors alerting").getAttribute("aria-checked")).toBe("false");
    expect(sectorIds(section(again.host, "ZDC Enroute sectors"))).toEqual(["ZDC05", "ZDC06"]);
    // Another table, and another facility, keep the defaults: filter on at 2 h.
    expect(control<HTMLInputElement>(again.host, "ZDC TRACON range").value).toBe("4");
    expect(control(again.host, "ZDC TRACON: only sectors alerting").getAttribute("aria-checked")).toBe("true");
    await setValue(control(again.host, "Facility"), "ZNY");
    expect(control<HTMLInputElement>(again.host, "ZNY Enroute range").value).toBe("4");
    expect(control(again.host, "ZNY Enroute: only sectors alerting").getAttribute("aria-checked")).toBe("true");
    expect(control<HTMLSelectElement>(again.host, "ZNY Enroute alert span").value).toBe("2");
  });

  it("opens filtered to the sectors alerting in the next 2 hours", async () => {
    // ZDC06 alerts at 1645Z (2h45 out), ZDC07 at 1545Z (1h45, the span's last bin), ZDC05 never.
    const enrouteRows = [row("ZDC05"), row("ZDC06", {11: "watch"}), row("ZDC07", {7: "over"})];
    const {host} = await mount(READER, [demand("ZDC", {enroute: {has_sector_data: true, rows: enrouteRows}})], {defaults: true});
    for (const table of ["ZDC Enroute", "ZDC TRACON"]) {
      expect(control(host, `${table}: only sectors alerting`).getAttribute("aria-checked"), table).toBe("true");
      expect(control<HTMLSelectElement>(host, `${table} alert span`).value, table).toBe("2");
    }
    expect(sectorIds(section(host, "ZDC Enroute sectors"))).toEqual(["ZDC07"]);
    expect(sectorIds(section(host, "ZDC TRACON sectors"))).toEqual(["ZDC80"]);
    // Nothing is hidden for good: switching it off draws every row, and the range is untouched.
    await click(control(host, "ZDC Enroute: only sectors alerting"));
    expect(sectorIds(section(host, "ZDC Enroute sectors"))).toEqual(["ZDC05", "ZDC06", "ZDC07"]);
    expect(levelCells(section(host, "ZDC Enroute sectors"))).toHaveLength(3 * 16);
  });

  it("says nothing is alerting, rather than drawing an empty grid, when the default hides every row", async () => {
    const {host} = await mount(READER, [demand("ZDC")], {defaults: true});
    // ZDC05 is quiet and ZDC06 alerts three hours out: neither is inside the default 2 h.
    const enroute = section(host, "ZDC Enroute sectors")!;
    expect(enroute.querySelector("table")).toBeNull();
    expect(enroute.textContent).toContain("No ZDC sectors alerting in the next 2.00 h");
  });

  it("still works when browser storage throws", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const {host} = await mount(READER, [demand("ZDC", {neighbours: ["ZNY"]}), demand("ZNY")]);
    // Nothing could be read, so the defaults apply: a 4 h range, filtered to the next 2 h.
    expect(control<HTMLInputElement>(host, "ZDC Enroute range").value).toBe("4");
    expect(control(host, "ZDC Enroute: only sectors alerting").getAttribute("aria-checked")).toBe("true");
    expect(sectorIds(section(host, "ZDC TRACON sectors"))).toEqual(["ZDC80"]);
    await setValue(control(host, "ZDC TRACON range"), "2");
    expect(levelCells(section(host, "ZDC TRACON sectors"))).toHaveLength(8);
    await click(control(host, "ZDC Enroute: only sectors alerting"));
    expect(sectorIds(section(host, "ZDC Enroute sectors"))).toEqual(["ZDC05", "ZDC06"]);
    await click(host.querySelector<HTMLElement>("button[aria-expanded]")!);
    expect(section(host, "ZNY Enroute sectors")).not.toBeNull();
  });

  it("labels a combined row with the sectors it carries", async () => {
    const {host} = await mount(READER, [
      demand("ZDC", {enroute: {has_sector_data: true, rows: [row("ZDC05", {}, {consolidated: ["ZDC06", "ZDC07"]})]}}),
    ]);
    const label = section(host, "ZDC Enroute sectors")!.querySelector<HTMLElement>("[data-carries]")!;
    expect(label.textContent).toBe("carries +ZDC06 +ZDC07");
    expect(sectorIds(section(host, "ZDC Enroute sectors"))).toEqual(["ZDC05"]);
  });

  it("edits the facility's own limits only where the server allows it", async () => {
    const own = await mount(READER, [demand("ZDC", {limits_editable: true})]);
    expect(own.host.querySelector('input[aria-label="Limit for ZDC05"]')).not.toBeNull();
    const readOnly = await mount(READER, [demand("ZDC")]);
    expect(readOnly.host.querySelector('input[aria-label="Limit for ZDC05"]')).toBeNull();
  });

  it("writes an edited limit to the facility's own route and refetches its demand", async () => {
    put.mockResolvedValue({data: {sector_id: "ZDC05", tier: "high", limit: 14, overridden: true}});
    const {host, qc} = await mount(READER, [demand("ZDC", {limits_editable: true})]);
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Limit for ZDC05"]')!;
    await act(async () => input.focus());
    await setValue(input, "14");
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true})));
    await act(async () => new Promise((r) => setTimeout(r, 0)));
    expect(put).toHaveBeenCalledWith("/api/v1/flow/sector-limits/{artcc}/{sector_id}", {
      params: {path: {artcc: "ZDC", sector_id: "ZDC05"}},
      body: {limit: 14},
    });
    expect(qc.getQueryState(sectorDemandKey("ZDC"))!.isInvalidated).toBe(true);
  });

  it("keeps a refused limit off the grid: the cell shows the stored value, a toast says so, nothing refetches", async () => {
    put.mockResolvedValue({error: {status: 403}});
    const {host, qc} = await mount(READER, [demand("ZDC", {limits_editable: true})]);
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Limit for ZDC05"]')!;
    await act(async () => input.focus());
    await setValue(input, "14");
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true})));
    await act(async () => new Promise((r) => setTimeout(r, 0)));
    expect(put).toHaveBeenCalledTimes(1);
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Limit for ZDC05"]')!.value).toBe("10");
    expect(document.body.textContent).toContain("Couldn't set ZDC05's limit");
    expect(qc.getQueryState(sectorDemandKey("ZDC"))!.isInvalidated).toBe(false);
  });

  // #722's deleted editor reset each input on its own key; the grid must too, or a refused write on one
  // sector throws away what the controller is typing into another.
  it("a refused write resets only its own input: a draft in another sector survives (#722)", async () => {
    put.mockResolvedValue({error: {status: 403}});
    const {host} = await mount(READER, [demand("ZDC", {limits_editable: true})]);
    const input = (id: string) => host.querySelector<HTMLInputElement>(`input[aria-label="Limit for ${id}"]`)!;
    // A draft held in ZDC06 (typed, not committed) …
    await setValue(input("ZDC06"), "17");
    // … while a write on ZDC05 is refused and raises a toast.
    await act(async () => input("ZDC05").focus());
    await setValue(input("ZDC05"), "20");
    await act(async () => input("ZDC05").dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true})));
    await act(async () => new Promise((r) => setTimeout(r, 0)));
    expect(put).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("Couldn't set ZDC05's limit");
    expect(input("ZDC05").value).toBe("10");
    expect(input("ZDC06").value).toBe("17");
  });

  // #722's limit editor was a separate table, never placed on a page; its cases now run against the
  // grid's inline input, which is where limits are edited (#725 owner decision 5). Each entry fails
  // "positive whole number" or equals the stored value, and none may reach the API or clear the override.
  describe("an inline limit entry that is not a change (#722)", () => {
    const overridden = () =>
      demand("ZDC", {limits_editable: true, enroute: {has_sector_data: true, rows: [row("ZDC05", {}, {limit: 14, limit_overridden: true})]}});
    const limitInput = (host: HTMLElement) => host.querySelector<HTMLInputElement>('input[aria-label="Limit for ZDC05"]')!;
    const key = (input: HTMLInputElement, k: string) =>
      act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: k, bubbles: true})));
    const settle = () => act(async () => new Promise((r) => setTimeout(r, 0)));

    it.each([
      ["zero", "0"],
      ["negative", "-3"],
      ["non-numeric", "abc"],
      ["empty", ""],
      ["fractional", "1.5"],
      ["unchanged", "14"],
    ])("cancels a %s entry on Enter and on blur: no PUT, no refetch, the override stands", async (_case, entry) => {
      const {host, qc} = await mount(READER, [overridden()]);
      for (const commit of [(i: HTMLInputElement) => key(i, "Enter"), (i: HTMLInputElement) => act(async () => i.blur())]) {
        const input = limitInput(host);
        await act(async () => input.focus());
        await setValue(input, entry);
        await commit(input);
        await settle();
        expect(put).not.toHaveBeenCalled();
        expect(limitInput(host).value).toBe("14");
      }
      expect(qc.getQueryState(sectorDemandKey("ZDC"))!.isInvalidated).toBe(false);
    });

    it("Escape discards a valid draft without a write", async () => {
      const {host} = await mount(READER, [overridden()]);
      const input = limitInput(host);
      await act(async () => input.focus());
      await setValue(input, "20");
      await key(input, "Escape");
      await settle();
      expect(put).not.toHaveBeenCalled();
      expect(limitInput(host).value).toBe("14");
    });

    it("setting the default over an override is a write (the server's reset), not a cancel", async () => {
      put.mockResolvedValue({data: {sector_id: "ZDC05", tier: "high", limit: 10, overridden: false}});
      const {host, qc} = await mount(READER, [overridden()]);
      const input = limitInput(host);
      await act(async () => input.focus());
      await setValue(input, "10");
      await act(async () => input.blur());
      await settle();
      expect(put).toHaveBeenCalledTimes(1);
      expect(put.mock.calls[0][1]).toMatchObject({params: {path: {artcc: "ZDC", sector_id: "ZDC05"}}, body: {limit: 10}});
      expect(qc.getQueryState(sectorDemandKey("ZDC"))!.isInvalidated).toBe(true);
    });
  });

  it("collapses neighbours by default, fetches one only when opened, and never offers its limits for editing", async () => {
    const {host} = await mount(READER, [
      demand("ZDC", {limits_editable: true, neighbours: ["ZNY", "ZOB"]}),
      // A national editor reads `limits_editable` for a neighbour too; the page must ignore it there.
      demand("ZNY", {limits_editable: true}),
    ]);
    const toggles = [...host.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")];
    expect(toggles.map((b) => b.textContent)).toEqual(["ZNYView only", "ZOBView only"]);
    expect(toggles.every((b) => b.getAttribute("aria-expanded") === "false")).toBe(true);
    expect(section(host, "ZNY Enroute sectors")).toBeNull();
    expect(demandRequests()).toEqual([]);

    await click(toggles[0]);
    const zny = section(host, "ZNY Enroute sectors");
    expect(sectorIds(zny)).toEqual(["ZNY05", "ZNY06"]);
    expect(host.querySelectorAll('input[aria-label^="Limit for ZNY"]')).toHaveLength(0);
    expect(host.querySelector('input[aria-label="Limit for ZDC05"]')).not.toBeNull();
    // ZOB stays closed and unasked.
    expect(demandRequests()).toEqual([]);

    await click(toggles[1]);
    expect(demandRequests().map(([, opts]) => opts.params.path.artcc)).toEqual(["ZOB"]);
  });

  it("remembers a neighbour's open state per browser per facility", async () => {
    const bodies = [demand("ZDC", {neighbours: ["ZNY"]}), demand("ZOB", {neighbours: ["ZNY"]}), demand("ZNY")];
    const first = await mount(READER, bodies);
    await click(first.host.querySelector<HTMLElement>("button[aria-expanded]")!);
    await first.unmount();

    const again = await mount(READER, bodies);
    expect(again.host.querySelector("button[aria-expanded]")!.getAttribute("aria-expanded")).toBe("true");
    // The same neighbour on another facility's page starts collapsed.
    await setValue(control(again.host, "Facility"), "ZOB");
    expect(again.host.querySelector("button[aria-expanded]")!.getAttribute("aria-expanded")).toBe("false");
  });

  it("draws each bin in the level the server judged it, never all green", async () => {
    // The server owns the alert rule (#722); the page must carry its verdict to the cell, or an
    // overload reads as a quiet sky. ZDC80 is red in its first bin; ZDC06 yellow at 1700Z.
    const {host} = await mount(READER, [demand("ZDC")]);
    const levels = (name: string) => [...levelCells(section(host, name))].map((c) => c.getAttribute("data-level"));
    const tracon = levels("ZDC TRACON sectors");
    expect(tracon[0]).toBe("over");
    expect(tracon.slice(1).every((l) => l === "ok")).toBe(true);
    const enroute = levels("ZDC Enroute sectors");
    // Row-major, 16 bins a row: ZDC05 green throughout, ZDC06 yellow at its bin 12 only.
    expect(enroute.slice(0, 16).every((l) => l === "ok")).toBe(true);
    expect(enroute.slice(16).map((l, i) => (l === "ok" ? null : `${i}:${l}`)).filter(Boolean)).toEqual(["12:watch"]);
  });

  it("keeps the three empty states distinct, none of them a grid", async () => {
    const empty = {cycle_at: null, bin_starts_ms: [], enroute: {has_sector_data: false, rows: []}, tracon: {has_sector_data: false, rows: []}};
    const {host} = await mount(READER, [
      demand("ZDC", {...empty, status: "pending", enroute: {has_sector_data: true, rows: []}, tracon: {has_sector_data: true, rows: []}}),
      demand("ZLA", {...empty, status: "no_sector_data"}),
      demand("ZNY"),
    ]);
    const shown = async (artcc: string) => {
      await setValue(control(host, "Facility"), artcc);
      return host.textContent ?? "";
    };
    const pending = await shown("ZDC");
    const noData = await shown("ZLA");
    expect(host.querySelector("table")).toBeNull();
    await shown("ZNY");
    await click(control(host, "ZNY Enroute: only sectors alerting"));
    await setValue(control(host, "ZNY Enroute alert span"), "1");
    const noneAlerting = section(host, "ZNY Enroute sectors")!.textContent ?? "";
    expect(section(host, "ZNY Enroute sectors")!.querySelector("table")).toBeNull();

    expect(pending).toContain("Waiting for the first cycle");
    expect(noData).toContain("No sector data for ZLA");
    expect(noneAlerting).toContain("No ZNY sectors alerting in the next 1.00 h");
    // Each says only its own thing.
    for (const [text, others] of [
      [pending, ["No sector data", "alerting in the next"]],
      [noData, ["Waiting for the first cycle", "alerting in the next"]],
      [noneAlerting, ["Waiting for the first cycle", "No sector data"]],
    ] as const) {
      for (const other of others) expect(text).not.toContain(other);
    }
  });

  it("replaces the whole set on a facility switch", async () => {
    const {host} = await mount(READER, [demand("ZDC", {neighbours: ["ZNY"]}), demand("ZOB", {neighbours: ["ZID"]}), demand("ZNY")]);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {});
    // Change ZDC's controls and open its neighbour first (storage swallowed, so only live state carries).
    await setValue(control(host, "ZDC Enroute range"), "2");
    await click(host.querySelector<HTMLElement>("button[aria-expanded]")!);
    expect(section(host, "ZNY Enroute sectors")).not.toBeNull();
    await setValue(control(host, "Facility"), "ZOB");
    expect(host.querySelector('section[aria-label^="ZDC"]')).toBeNull();
    expect(section(host, "ZNY Enroute sectors")).toBeNull();
    expect(sectorIds(section(host, "ZOB Enroute sectors"))).toEqual(["ZOB05", "ZOB06"]);
    // ZOB's tables start from their own state, not ZDC's.
    expect(control<HTMLInputElement>(host, "ZOB Enroute range").value).toBe("4");
    // ZDC's neighbour is gone with it; only ZOB's own neighbour is listed, collapsed.
    const toggles = [...host.querySelectorAll("button[aria-expanded]")];
    expect(toggles.map((b) => b.textContent)).toEqual(["ZIDView only"]);
    expect(toggles[0].getAttribute("aria-expanded")).toBe("false");
  });

  it("never remembers the facility pick: a reload opens on the viewer's home facility", async () => {
    const bodies = [demand("ZDC"), demand("ZOB"), demand("ZLA")];
    const first = await mount(READER, bodies);
    await setValue(control(first.host, "Facility"), "ZOB");
    expect(section(first.host, "ZOB Enroute sectors")).not.toBeNull();
    await first.unmount();

    const again = await mount(READER, bodies);
    expect(control<HTMLSelectElement>(again.host, "Facility").value).toBe("ZDC");
    expect(again.host.querySelector('section[aria-label^="ZOB"]')).toBeNull();
    await again.unmount();

    // A controller who moved from ZDC to ZLA opens on ZLA, with no ZDC table left over.
    const moved = await mount(user({flow: {sectors: ["read"]}}, "ZLA"), bodies);
    expect(control<HTMLSelectElement>(moved.host, "Facility").value).toBe("ZLA");
    expect(moved.host.querySelector('section[aria-label^="ZDC"]')).toBeNull();
    expect(Object.keys(localStorage).filter((k) => !k.startsWith("ois.sectorDemand.view.") && !k.startsWith("ois.sectorDemand.open."))).toEqual([]);
  });
});

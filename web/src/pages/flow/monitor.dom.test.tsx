// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const post = vi.hoisted(() => vi.fn());
// The generated client captures `fetch` at load, so mock the client and seed the cache (#387).
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: post, PUT: vi.fn(), DELETE: vi.fn()}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));
vi.mock("@/lib/admin", () => ({useFacilities: () => ({data: [{id: "ZDC", name: "Washington", active: true}]})}));

import {MonitorPage, MonitorTable} from "./monitor";
import {
  type MonitorBin,
  type MonitorRow,
  type MonitorTable as Table,
  monitorKey,
  monitorNeighboursKey,
} from "@/lib/monitor";

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
  get.mockReset();
  post.mockReset();
});

const AS_OF = "2026-10-04T14:07:00Z";
const FIRST = Date.parse("2026-10-04T14:00:00Z");

function row(id: string, alertNow: "green" | "amber" | "red"): MonitorRow {
  const bins: MonitorBin[] = Array.from({length: 24}, (_, i) => ({
    start: new Date(FIRST + i * 15 * 60_000).toISOString(),
    active: 0,
    proposed: 0,
    combined: i === 0 ? 12 : 0,
    alert: i === 0 ? alertNow : "green",
  }));
  return {sector_id: id, name: null, map: 10, consolidated: [], staffed: false, bins};
}

async function mount(table: Table, viewOnly = false) {
  const qc = new QueryClient({defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}}});
  qc.setQueryData(monitorKey("ZDC"), table);
  get.mockResolvedValue({data: table});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <MonitorTable artcc="ZDC" viewOnly={viewOnly} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

const table = (editable: boolean, rows: MonitorRow[]): Table => ({artcc: "ZDC", editable, as_of: AS_OF, rows});

async function rightClickFirstRow(host: HTMLElement) {
  const tr = host.querySelector("tbody tr")!;
  await act(async () => {
    tr.dispatchEvent(new MouseEvent("contextmenu", {bubbles: true, cancelable: true, clientX: 10, clientY: 10}));
  });
}

describe("Airspace Monitor page (#601)", () => {
  /** AC6: without edit rights the MAP is plain text and a right-click opens nothing. */
  it("leaves the MAP cell and row menu inert without edit rights", async () => {
    const host = await mount(table(false, [row("02", "red"), row("03", "amber")]));
    expect(host.querySelector('input[aria-label="02 alert parameter"]')).toBeNull();
    expect(host.textContent).toContain("10");
    await rightClickFirstRow(host);
    expect(document.querySelector('[role="menuitem"]')).toBeNull();
  });

  it("makes them live with edit rights", async () => {
    const host = await mount(table(true, [row("02", "red"), row("03", "amber")]));
    expect(host.querySelector('input[aria-label="02 alert parameter"]')).not.toBeNull();
    await rightClickFirstRow(host);
    const items = [...document.querySelectorAll('[role="menuitem"]')].map((el) => el.textContent);
    expect(items).toContain("Consolidate into 03");
  });

  /** #713: the bulk consolidations, for the right-clicked row, as one request each. */
  it("offers both bulk consolidations and sends the chosen one in one request", async () => {
    post.mockResolvedValue({response: {ok: true}});
    const host = await mount(table(true, [row("02", "red"), row("03", "amber")]));
    await rightClickFirstRow(host);
    const items = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')];
    expect(items.map((el) => el.textContent)).toEqual(
      expect.arrayContaining(["All into 02", "All into 02 except consolidated"]),
    );
    await act(async () => {
      items.find((el) => el.textContent === "All into 02 except consolidated")!.click();
    });
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/api/v1/flow/monitor/{artcc}/consolidations", {
      params: {path: {artcc: "ZDC"}},
      body: {target_sector_id: "02", mode: "except_consolidated"},
    });
  });

  it("says when there is no sector data yet", async () => {
    const host = await mount(table(false, []));
    expect(host.textContent).toContain("No sector data yet");
  });

  /**
   * AC4, where it is wired: the filter judges a row on all six hours, not on the Time Range's slice.
   * Only a filter wider than the table tells the two apart — a 2-hour table filtering on the next 3
   * hours must keep a sector that goes red at +2.5 h, a bin the table doesn't draw (#601 review).
   */
  it("keeps a row whose alert is past the Time Range but inside the filter", async () => {
    localStorage.setItem("ois.monitor.ZDC.range", JSON.stringify(2));
    localStorage.setItem("ois.monitor.ZDC.alert", JSON.stringify(3));
    const later = row("02", "green");
    later.bins[10] = {...later.bins[10], combined: 12, alert: "red"}; // 16:30Z, 2h23m after as_of
    const host = await mount(table(false, [later, row("03", "green")]));
    const shown = [...host.querySelectorAll("tbody tr")].map((tr) => tr.textContent);
    expect(shown).toHaveLength(1);
    expect(shown[0]).toContain("02");
  });

  it("says when the alert filter hides every row", async () => {
    const host = await mount(table(false, [row("02", "green")]));
    expect(host.textContent).toContain("Nothing alerting");
    expect(host.querySelector("tbody")).toBeNull();
  });

  /** #712 AC2: a neighbour's table is view-only even where the server would let this caller edit. */
  it("keeps a neighbour's table inert even for an editor", async () => {
    const host = await mount(table(true, [row("02", "red"), row("03", "amber")]), true);
    expect(host.querySelector('input[aria-label="02 alert parameter"]')).toBeNull();
    await rightClickFirstRow(host);
    expect(document.querySelector('[role="menuitem"]')).toBeNull();
  });

  /** #712 AC1: the selected ARTCC's table first, then each neighbour collapsed until opened. */
  it("shows the ARTCC's table and its neighbours collapsed", async () => {
    localStorage.setItem("ois.monitor.artcc", JSON.stringify("ZDC"));
    const qc = new QueryClient({defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}}});
    qc.setQueryData(monitorKey("ZDC"), table(false, [row("02", "red")]));
    // The server would let this caller edit ZNY (a national editor): the neighbour must stay inert anyway.
    qc.setQueryData(monitorKey("ZNY"), {...table(true, [row("10", "red")]), artcc: "ZNY"});
    qc.setQueryData(monitorNeighboursKey("ZDC"), ["ZNY"]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <ToastProvider>
            <MonitorPage />
          </ToastProvider>
        </QueryClientProvider>,
      );
    });
    const toggle = [...host.querySelectorAll("button")].find((b) => b.textContent === "ZNY")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelectorAll("table")).toHaveLength(1);
    await act(async () => toggle.click());
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelectorAll("table")).toHaveLength(2);
    expect(host.querySelector('input[aria-label="10 alert parameter"]')).toBeNull();
  });
});

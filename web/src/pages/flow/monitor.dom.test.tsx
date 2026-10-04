// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
// The generated client captures `fetch` at load, so mock the client and seed the cache (#387).
vi.mock("@/lib/api", () => ({ois: {GET: get, PUT: vi.fn(), DELETE: vi.fn()}}));

import {MonitorTable} from "./monitor";
import {type MonitorBin, type MonitorRow, type MonitorTable as Table, monitorKey} from "@/lib/monitor";

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

async function mount(table: Table) {
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
          <MonitorTable artcc="ZDC" />
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

  it("says when there is no sector data yet", async () => {
    const host = await mount(table(false, []));
    expect(host.textContent).toContain("No sector data yet");
  });

  it("says when the alert filter hides every row", async () => {
    const host = await mount(table(false, [row("02", "green")]));
    expect(host.textContent).toContain("Nothing alerting");
    expect(host.querySelector("tbody")).toBeNull();
  });
});

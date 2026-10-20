// @vitest-environment jsdom
// Coverage matrix (#824, AC8 and AC9 on the dashboards library)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC8 armed Delete survives a collections change | keeps an armed Delete armed when the collections ...    | the actions `cell` closes over `collections`
//                                              |                                                          | (columns memo keyed on it)
// AC9 failed refetch keeps table + armed button | keeps the table and an armed Delete when a background ... | QueryState gets `isError` without `&& !data`
// AC9 control: error with no data              | shows the error state when the first load fails          | the gate drops `isError` altogether
// AC9 a 5xx refetch                            | keeps the table and an armed Delete when a background refetch answers 500 | useDashboards returns an empty library on an error response
import type * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider, ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: vi.fn(), PATCH: vi.fn(), DELETE: del}}));
// The page needs a router only for links and navigation, neither of which this file exercises.
vi.mock("@tanstack/react-router", () => ({
  Link: ({children}: {children?: React.ReactNode}) => <a>{children}</a>,
  useNavigate: () => () => {},
}));
// The list view is the one that renders a DataTable; the grid view uses cards.
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}, useView: () => "list"}));

import {BoardLibraryPage} from "./library";
import type {DashboardLibrary} from "@/lib/dashboards";
import type {Me} from "@/lib/auth";

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
  get.mockReset();
  del.mockReset();
});

const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Controller",
  rating: null,
  primary_role: null,
  server_admin: false,
  tmu_national: false,
  permissions: {},
} as unknown as Me;

const LIBRARY: DashboardLibrary = {
  dashboards: [{id: "b1", name: "ZDC overview", collection_id: "c1", updated_at: "2026-10-01T00:00:00Z"}],
  collections: [{id: "c1", name: "Arrivals"}],
} as DashboardLibrary;

async function mount(seed: DashboardLibrary | null) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
    },
  });
  if (seed) qc.setQueryData(["dashboards"], seed);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <DialogProvider>
            <BoardLibraryPage />
          </DialogProvider>
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** The row's delete control is an icon-only ConfirmButton: the one row button with no text.
 *  Re-queried on every call, because a remounted cell replaces the DOM node. */
function deleteButton(host: HTMLElement) {
  const el = [...host.querySelectorAll("tbody button")].find((b) => !b.textContent?.trim());
  if (!el) throw new Error("no delete button in the row");
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("An armed Delete in the library list survives a collections change (#824 AC8)", () => {
  it("keeps an armed Delete armed when the collections list changes between the two clicks", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount(LIBRARY);

    await click(deleteButton(host));
    expect(toasts().some((t) => t.includes('Delete "ZDC overview"?'))).toBe(true);
    expect(del).not.toHaveBeenCalled();

    // A real refetch through the hook's queryFn: a collection was added and the board moved into it
    // (another tab used Move). The board must change too: with `dashboards` deep-equal, structural
    // sharing keeps its reference and the table keeps each row's cached Collection value, so the
    // positive control below could not see the refetch.
    get.mockResolvedValue({
      data: {
        dashboards: [{...LIBRARY.dashboards[0]!, collection_id: "c2"}],
        collections: [
          {id: "c1", name: "Arrivals"},
          {id: "c2", name: "Departures"},
        ],
      },
      error: undefined,
      response: {status: 200},
    });
    await act(async () => {
      await qc.refetchQueries({queryKey: ["dashboards"]});
    });
    await settle();
    // Positive control: the Collection column re-rendered with the new collection before the second
    // click.
    expect(get).toHaveBeenCalledWith("/api/v1/dashboards");
    expect(host.querySelector("tbody")?.textContent).toContain("Departures");

    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/dashboards/{id}", {params: {path: {id: "b1"}}});
  });
});

describe("A failed background refetch keeps the library table and an armed Delete (#824 AC9)", () => {
  // `ois.GET` rejecting is a network failure: `useDashboards`'s queryFn throws, the query goes to
  // `error`, and TanStack Query keeps the last data. That is the state the page must render through.
  it("keeps the table and an armed Delete when a background refetch fails", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount(LIBRARY);

    await click(deleteButton(host));
    expect(del).not.toHaveBeenCalled();

    get.mockRejectedValue(new TypeError("Failed to fetch"));
    await act(async () => {
      await qc.refetchQueries({queryKey: ["dashboards"]});
    });
    await settle();
    // Positive control: the refetch really failed, and the cache still holds the rows.
    expect(qc.getQueryState(["dashboards"])?.status).toBe("error");
    expect(qc.getQueryData<DashboardLibrary>(["dashboards"])?.dashboards).toHaveLength(1);

    expect(host.querySelector("table"), "the table stays mounted").not.toBeNull();
    expect(host.textContent).not.toContain("Couldn't load this.");
    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/dashboards/{id}", {params: {path: {id: "b1"}}});
  });

  // The other half of the gate: with nothing loaded, an error still shows the error state.
  it("shows the error state when the first load fails with nothing loaded", async () => {
    get.mockRejectedValue(new TypeError("Failed to fetch"));
    const {host, qc} = await mount(null);
    await settle();

    expect(qc.getQueryState(["dashboards"])?.status).toBe("error");
    expect(host.textContent).toContain("Couldn't load this.");
    expect(host.querySelector("table")).toBeNull();
  });

  // An error response, not only a network failure: `useDashboards` throws on it, so the query goes to
  // `error` with the rows kept instead of "succeeding" with an empty library.
  it("keeps the table and an armed Delete when a background refetch answers 500", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount(LIBRARY);

    await click(deleteButton(host));
    expect(del).not.toHaveBeenCalled();

    get.mockResolvedValue({data: undefined, error: {error: "internal"}, response: {status: 500}});
    await act(async () => {
      await qc.refetchQueries({queryKey: ["dashboards"]});
    });
    await settle();
    // Positive control: the refetch ran and failed, and the cache still holds the rows.
    expect(get).toHaveBeenCalledWith("/api/v1/dashboards");
    expect(qc.getQueryState(["dashboards"])?.status).toBe("error");
    expect(qc.getQueryData<DashboardLibrary>(["dashboards"])?.dashboards).toHaveLength(1);

    expect(host.textContent).not.toContain("No dashboards yet");
    expect(host.querySelector("table"), "the table stays mounted").not.toBeNull();
    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
  });
});

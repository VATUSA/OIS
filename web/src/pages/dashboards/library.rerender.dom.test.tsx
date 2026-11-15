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
// Move menu lists the refetched collections     | offers every collection in a row's Move menu after ... | the row menu stops reading LibraryCollections, or
//                                              |                                                          | the Provider is dropped or moved below the table
// Rename and Move still reach the board          | (same test) and renames a board from its row ...       | onMove / onRename lose their wiring in actionsFor
//
// Coverage matrix (#877, the Collection column after a collection rename)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC1/AC3 renamed collection shows in its row  | shows a collection's new name in its row when only ...   | the Collection column resolves the name in an
//                                              |                                                          | accessorFn over `collections` (the row model
//                                              |                                                          | is memoized on `data`, which did not change)
// AC2 ascending sort orders by the new name    | sorts by the new collection names, ascending, after ...  | the sort reads the per-row cached accessor value
// AC2 descending sort orders by the new name   | sorts by the new collection names, descending, after ... | as above, on the second header click
// No collection, or a missing one, shows "—"   | shows a dash for a board with no collection and for ...  | the row's name falls back to anything but ""
import type * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider, ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: put, PATCH: vi.fn(), DELETE: del}}));
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
  put.mockReset();
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
    // (another tab used Move), so the positive control below sees the refetch through a changed row.
    // A refetch that changes only the collections is the #877 tests' case.
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

describe("The list view's row menu still files and renames boards (#824)", () => {
  /** The row's Move or Rename button, by its text. */
  function rowButton(host: HTMLElement, label: string) {
    const el = [...host.querySelectorAll("tbody button")].find((b) => b.textContent?.trim() === label);
    if (!el) throw new Error(`no ${label} button in the row`);
    return el;
  }

  it("offers every collection in a row's Move menu after the collections refetch, and files the board", async () => {
    put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
    const {host, qc} = await mount(LIBRARY);

    // The collections change after mount, so the menu can only list "Departures" if it reads the
    // current collections through the context.
    get.mockResolvedValue({
      data: {
        dashboards: LIBRARY.dashboards,
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

    await act(async () => {
      // Radix opens the menu on pointerdown, which jsdom does not synthesise from a click.
      rowButton(host, "Move").dispatchEvent(
        new MouseEvent("pointerdown", {bubbles: true, cancelable: true, button: 0}),
      );
      await new Promise((r) => setTimeout(r, 0));
    });
    const items = () => [...document.querySelectorAll('[role="menuitem"]')];
    expect(items().map((el) => el.textContent?.trim())).toEqual(["No collection", "Arrivals", "Departures"]);

    const departures = items().find((el) => el.textContent?.trim() === "Departures")!;
    await act(async () => {
      departures.dispatchEvent(new MouseEvent("pointerdown", {bubbles: true, cancelable: true}));
      departures.dispatchEvent(new MouseEvent("pointerup", {bubbles: true, cancelable: true}));
      departures.dispatchEvent(new MouseEvent("click", {bubbles: true, cancelable: true}));
    });
    expect(put).toHaveBeenCalledTimes(1);
    expect(put).toHaveBeenCalledWith("/api/v1/dashboards/{id}", {
      params: {path: {id: "b1"}},
      body: {name: undefined, data: undefined, collection_id: "c2"},
    });
  });

  it("renames a board from its row through the rename prompt", async () => {
    put.mockResolvedValue({data: {}, error: undefined, response: {status: 200}});
    const {host} = await mount(LIBRARY);

    await click(rowButton(host, "Rename"));
    const input = document.querySelector<HTMLInputElement>('[role="dialog"] input');
    if (!input) throw new Error("the rename prompt did not open");
    expect(input.value).toBe("ZDC overview");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "ZDC arrivals");
      input.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => {
      input.form!.requestSubmit();
    });
    await settle();

    expect(put).toHaveBeenCalledTimes(1);
    expect(put).toHaveBeenCalledWith("/api/v1/dashboards/{id}", {
      params: {path: {id: "b1"}},
      body: {name: "ZDC arrivals", data: undefined, collection_id: undefined},
    });
  });
});

describe("The Collection column follows a collection rename (#877)", () => {
  /** A refetch through the hook's queryFn that renames collections and leaves every board as seeded,
   *  so structural sharing hands the table the same `dashboards` array. */
  async function refetchWith(qc: QueryClient, data: DashboardLibrary) {
    get.mockResolvedValue({data, error: undefined, response: {status: 200}});
    await act(async () => {
      await qc.refetchQueries({queryKey: ["dashboards"]});
    });
    await settle();
  }

  it("shows a collection's new name in its row when only the collection was renamed", async () => {
    const {host, qc} = await mount(LIBRARY);
    expect(host.querySelector("tbody")?.textContent).toContain("Arrivals");

    await refetchWith(qc, {
      dashboards: LIBRARY.dashboards.map((b) => ({...b})),
      collections: [{id: "c1", name: "Holding"}],
    } as DashboardLibrary);
    // Positive control: the refetch landed, and structural sharing kept the seeded boards array.
    expect(get).toHaveBeenCalledWith("/api/v1/dashboards");
    const cached = qc.getQueryData<DashboardLibrary>(["dashboards"]);
    expect(cached?.collections[0]?.name).toBe("Holding");
    expect(cached?.dashboards).toBe(LIBRARY.dashboards);

    const row = host.querySelector("tbody")?.textContent ?? "";
    expect(row).toContain("Holding");
    expect(row).not.toContain("Arrivals");
  });

  it("shows a dash for a board with no collection and for one whose collection is gone", async () => {
    const {host, qc} = await mount({
      dashboards: [
        {id: "b1", name: "Ungrouped board", collection_id: null, updated_at: "2026-10-02T00:00:00Z"},
        {id: "b2", name: "Orphaned board", collection_id: "gone", updated_at: "2026-10-01T00:00:00Z"},
        {id: "b3", name: "Filed board", collection_id: "c1", updated_at: "2026-09-30T00:00:00Z"},
      ],
      collections: [{id: "c1", name: "Arrivals"}],
    } as DashboardLibrary);
    await refetchWith(qc, {
      dashboards: (qc.getQueryData<DashboardLibrary>(["dashboards"])?.dashboards ?? []).map((b) => ({...b})),
      collections: [{id: "c1", name: "Holding"}],
    } as DashboardLibrary);

    const cells = [...host.querySelectorAll("tbody tr")].map((tr) =>
      [...tr.querySelectorAll("td")].slice(0, 2).map((td) => td.textContent?.trim()),
    );
    // Positive control: the filed board picked up the rename in the same render.
    expect(cells).toEqual([
      ["Ungrouped board", "—"],
      ["Orphaned board", "—"],
      ["Filed board", "Holding"],
    ]);
  });

  // b1 sits in "Arrivals" and b2 in "Departures"; renaming c1 to "Zulu" flips their order by name, so
  // a sort over the old names puts b1 and b2 the other way round. b3 has no collection (an empty name)
  // and is the newest, so the page's starting order, b3 b1 b2, is neither sorted result.
  const THREE = {
    dashboards: [
      {id: "b3", name: "Board three", collection_id: null, updated_at: "2026-10-03T00:00:00Z"},
      {id: "b1", name: "Board one", collection_id: "c1", updated_at: "2026-10-02T00:00:00Z"},
      {id: "b2", name: "Board two", collection_id: "c2", updated_at: "2026-10-01T00:00:00Z"},
    ],
    collections: [
      {id: "c1", name: "Arrivals"},
      {id: "c2", name: "Departures"},
    ],
  } as DashboardLibrary;
  const renamed = () =>
    ({
      dashboards: THREE.dashboards.map((b) => ({...b})),
      collections: [
        {id: "c1", name: "Zulu"},
        {id: "c2", name: "Departures"},
      ],
    }) as DashboardLibrary;

  function collectionHeader(host: HTMLElement) {
    const th = [...host.querySelectorAll("thead th")].find((h) => h.textContent?.trim() === "Collection");
    if (!th) throw new Error("no Collection header");
    return th;
  }
  const sortCollection = (host: HTMLElement) => click(collectionHeader(host).querySelector("button")!);
  const rowNames = (host: HTMLElement) =>
    [...host.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("td")?.textContent?.trim());

  it("sorts by the new collection names, ascending, after a collection rename", async () => {
    const {host, qc} = await mount(THREE);
    await refetchWith(qc, renamed());
    expect(qc.getQueryData<DashboardLibrary>(["dashboards"])?.dashboards).toBe(THREE.dashboards);

    await sortCollection(host);
    expect(collectionHeader(host).getAttribute("aria-sort")).toBe("ascending");
    // No collection, then "Departures", then "Zulu"; the old names would put "Arrivals" (b1) second.
    expect(rowNames(host)).toEqual(["Board three", "Board two", "Board one"]);
    expect(host.querySelector("tbody")?.textContent).not.toContain("Arrivals");
  });

  it("sorts by the new collection names, descending, after a collection rename", async () => {
    const {host, qc} = await mount(THREE);
    await refetchWith(qc, renamed());
    expect(qc.getQueryData<DashboardLibrary>(["dashboards"])?.dashboards).toBe(THREE.dashboards);

    await sortCollection(host);
    await sortCollection(host);
    expect(collectionHeader(host).getAttribute("aria-sort")).toBe("descending");
    // "Zulu", then "Departures", then no collection; the old names would put "Departures" (b2) first.
    expect(rowNames(host)).toEqual(["Board one", "Board two", "Board three"]);
  });
});

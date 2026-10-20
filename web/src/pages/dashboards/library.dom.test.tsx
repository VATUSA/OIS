// @vitest-environment jsdom
import type * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider, ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const del = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (VATUSA/OIS#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: vi.fn(), POST: vi.fn(), PUT: vi.fn(), DELETE: del}}));
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
  del.mockReset();
});

// The page reads `useMe` only for `tmu_national`, which picks the board templates it offers.
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
  dashboards: [{id: "b1", name: "ZDC overview", collection_id: null, updated_at: "2026-10-01T00:00:00Z"}],
  collections: [],
};

async function mount() {
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
  qc.setQueryData(["dashboards"], LIBRARY);
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

describe("Deleting a board from the list view confirms on the second click (#824)", () => {
  // The list view's columns were rebuilt on every render, so any re-render of the page between the
  // two clicks (here a refetch that renamed the board) remounted the armed button.
  it("sends the DELETE when the library refetches between the two clicks", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount();

    await click(deleteButton(host));
    expect(toasts().some((t) => t.includes('Delete "ZDC overview"?'))).toBe(true);
    await act(async () => {
      qc.setQueryData(["dashboards"], {
        ...LIBRARY,
        dashboards: [{...LIBRARY.dashboards[0]!, name: "ZDC overview v2"}],
      });
    });
    await settle();
    // Positive control: the page re-rendered with the refetched row before the second click.
    expect(host.querySelector("tbody")?.textContent).toContain("ZDC overview v2");
    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/dashboards/{id}", {params: {path: {id: "b1"}}});
  });
});

// @vitest-environment jsdom
// Coverage matrix (#824, AC9 on aircraft profiles)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC9 failed refetch keeps table + armed button | keeps the table and an armed Delete when a background ... | QueryState gets `profiles.isError` without
//                                              |                                                          | `&& !profiles.data`
// AC9 control: error with no data              | shows the error state when the first load fails ...      | the gate drops `isError` altogether
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: vi.fn(), DELETE: del}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {AircraftProfilesPage} from "./aircraft-profiles";
import type {AircraftProfile} from "@/lib/aircraft-profiles";
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

// A real permission tree: `update` is what adds the actions column with the ConfirmButton.
const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Controller",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {flow: {aircraft_profiles: ["read", "update"]}},
} as unknown as Me;

const B738: AircraftProfile = {
  kind: "type",
  key: "B738",
  name: "Boeing 737-800",
  climb_ias_lo: 250,
  climb_ias_hi: 280,
  climb_mach: 0.78,
  climb_fpm_lo: 2500,
  climb_fpm_hi: 1500,
  cruise_mach: 0.78,
  cruise_tas: null,
  desc_mach: 0.78,
  desc_ias_hi: 280,
  desc_ias_lo: 250,
  desc_fpm: 2000,
  service_ceiling_ft: 41000,
  updated_at: "2026-10-01T00:00:00Z",
  updated_by: null,
};

async function mount(seed: AircraftProfile[] | null) {
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
  if (seed) qc.setQueryData(["aircraft-profiles"], seed);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AircraftProfilesPage />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** Re-queried on every call: a remounted cell replaces the DOM node, so a held reference would lie. */
function deleteButton(host: HTMLElement) {
  const el = host.querySelector('tbody button[aria-label="Delete B738"]');
  if (!el) throw new Error("no Delete B738 button");
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("A failed background refetch keeps the aircraft profiles table (#824 AC9)", () => {
  it("keeps the table and an armed Delete when a background refetch fails", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount([B738]);

    await click(deleteButton(host));
    expect(toasts().some((t) => t.includes('Delete "B738"?'))).toBe(true);
    expect(del).not.toHaveBeenCalled();

    // A real refetch through the hook's queryFn, against an error response: it throws, the query goes
    // to `error`, and TanStack Query keeps the rows.
    get.mockResolvedValue({data: undefined, error: {error: "unavailable"}, response: {status: 503}});
    await act(async () => {
      await qc.refetchQueries({queryKey: ["aircraft-profiles"]});
    });
    await settle();
    // Positive control: the refetch really failed, and the cache still holds the row.
    expect(get).toHaveBeenCalledWith("/api/v1/flow/aircraft-profiles");
    expect(qc.getQueryState(["aircraft-profiles"])?.status).toBe("error");
    expect(qc.getQueryData<AircraftProfile[]>(["aircraft-profiles"])).toHaveLength(1);

    expect(host.querySelector("table"), "the table stays mounted").not.toBeNull();
    expect(host.textContent).not.toContain("Couldn't load this.");
    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/flow/aircraft-profiles/{kind}/{key}", {
      params: {path: {kind: "type", key: "B738"}},
    });
  });

  // The other half of the gate: with nothing loaded, an error still shows the error state.
  it("shows the error state when the first load fails with nothing loaded", async () => {
    get.mockResolvedValue({data: undefined, error: {error: "unavailable"}, response: {status: 503}});
    const {host, qc} = await mount(null);
    await settle();

    expect(qc.getQueryState(["aircraft-profiles"])?.status).toBe("error");
    expect(host.textContent).toContain("Couldn't load this.");
    expect(host.querySelector("table")).toBeNull();
  });
});

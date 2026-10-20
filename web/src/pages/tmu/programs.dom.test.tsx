// @vitest-environment jsdom
// Coverage matrix (#824, AC9 on TMU programs)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC9 failed refetch keeps table + armed button | keeps the table and an armed Remove when a background ... | QueryState gets `programs.isError` without
//                                              |                                                          | `&& !programs.data`
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
// The table view's Edit navigates; this file never clicks it. A stable function, because the
// table's columns memo depends on it.
const navigate = vi.hoisted(() => () => {});
vi.mock("@tanstack/react-router", () => ({useNavigate: () => navigate}));
vi.mock("@/components/shell/page-meta", () => ({useView: () => "table"}));

import {ProgramsTab} from "./programs";
import type {Me} from "@/lib/auth";
import type {Program} from "@/lib/tmu";

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

// A real permission tree: `delete` alone renders Remove, and leaving out `update` keeps the
// set-program form (and its own queries) off the page.
const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Controller",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {tmu: {program: ["read", "delete"]}},
} as unknown as Me;

const KIAD: Program = {
  icao: "KIAD",
  aar: 30,
  trail: 0,
  mit: 0,
  gates: [],
  exclude_types: [],
  exclude_wake: [],
  jets_only: false,
  active_until: null,
  updated_at: "2026-10-01T00:00:00Z",
  updated_by: null,
};

const PROGRAMS_PATH = "/api/v1/tmu/programs";
/** The programs GET answers `programs`; every other GET (the row's live-demand flow) fails, which the
 *  row renders as "live demand…". */
function answerPrograms(programs: () => Promise<unknown>) {
  get.mockImplementation((path: string) =>
    path === PROGRAMS_PATH
      ? programs()
      : Promise.resolve({data: undefined, error: {error: "x"}, response: {status: 500}}),
  );
}

async function mount(seed: Program[] | null) {
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
  if (seed) qc.setQueryData(["tmu-programs"], seed);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <ProgramsTab />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** Re-queried on every call: a remounted cell replaces the DOM node, so a held reference would lie. */
function removeButton(host: HTMLElement) {
  const el = [...host.querySelectorAll("tbody button")].find((b) => b.textContent?.trim() === "Remove");
  if (!el) throw new Error("no Remove button in the table");
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("A failed background refetch keeps the TMU programs table (#824 AC9)", () => {
  it("keeps the table and an armed Remove when a background refetch fails", async () => {
    answerPrograms(() => Promise.resolve({data: [KIAD], error: undefined, response: {status: 200}}));
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount([KIAD]);

    await click(removeButton(host));
    expect(toasts().some((t) => t.includes("Remove the KIAD program?"))).toBe(true);
    expect(del).not.toHaveBeenCalled();

    // A real refetch through the hook's queryFn, against an error response: it throws, the query goes
    // to `error`, and TanStack Query keeps the rows.
    answerPrograms(() => Promise.resolve({data: undefined, error: {error: "unavailable"}, response: {status: 503}}));
    await act(async () => {
      await qc.refetchQueries({queryKey: ["tmu-programs"]});
    });
    await settle();
    // Positive control: the refetch really failed, and the cache still holds the row.
    expect(get).toHaveBeenCalledWith(PROGRAMS_PATH);
    expect(qc.getQueryState(["tmu-programs"])?.status).toBe("error");
    expect(qc.getQueryData<Program[]>(["tmu-programs"])).toHaveLength(1);

    expect(host.querySelector("table"), "the table stays mounted").not.toBeNull();
    expect(host.textContent).not.toContain("Couldn't load programs.");
    await click(removeButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/tmu/programs/{icao}", {params: {path: {icao: "KIAD"}}});
  });

  // The other half of the gate: with nothing loaded, an error still shows the error state.
  it("shows the error state when the first load fails with nothing loaded", async () => {
    answerPrograms(() => Promise.resolve({data: undefined, error: {error: "unavailable"}, response: {status: 503}}));
    const {host, qc} = await mount(null);
    await settle();

    expect(qc.getQueryState(["tmu-programs"])?.status).toBe("error");
    expect(host.textContent).toContain("Couldn't load programs.");
    expect(host.querySelector("table")).toBeNull();
  });
});

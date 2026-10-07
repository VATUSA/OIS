// @vitest-environment jsdom
//
// VATUSA/OIS#789. The /ops/fca ARTCC filter opened on ALL ARTCCs on every visit, so a ZDC controller
// re-picked ZDC each time. It is now saved in the account's `fca` preferences namespace and restored
// on load, with the map framed to it. These mount the real FcaMapView against a real query cache, so
// going back to a bare `useState("")` fails the seeded-ZDC case.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const api = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn(), POST: vi.fn(), DELETE: vi.fn() }));
vi.mock("@/lib/api", () => ({ ois: api, API_BASE: "", DOCS_URL: "" }));

const camera = vi.hoisted(() => ({
  viewState: { longitude: -98, latitude: 39, zoom: 3.9 },
  onViewStateChange: () => {},
  onResize: () => {},
  flyTo: vi.fn(),
  fitBounds: vi.fn(),
  home: vi.fn(),
}));
vi.mock("./hooks/useMapCamera", () => ({ useMapCamera: () => camera }));
// deck.gl and the side panels aren't what's under test, and deck can't run in jsdom.
vi.mock("./TrafficMap", () => ({ TrafficMap: () => null }));
vi.mock("@/pages/fca/detail", () => ({ FcaDetail: () => null }));
vi.mock("@/pages/fca/overview-panel", () => ({ FcaOverviewPanel: () => null }));
vi.mock("@/components/flight-search", () => ({ FlightSearch: () => null }));
vi.mock("@tanstack/react-router", () => ({ Link: () => null }));

import type {Me} from "@/lib/auth";
import type {Fca} from "@/lib/fca";
import {FcaMapView} from "./FcaMapView";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const PREFS_PATH = "/api/v1/me/preferences/{namespace}";
const CONTROLLER = {
  server_admin: false,
  permissions: { flow: { fca: ["read", "update", "delete"] } },
} as unknown as Me;
const fca = (id: string, artcc: string) =>
  ({ id, name: id, color: "#efc14d", artcc, enabled: true, fixes: [], points: [] }) as unknown as Fca;
/** No ZDC FCA today: the saved ZDC must survive anyway. */
const FCAS = [fca("ny1", "ZNY"), fca("ob1", "ZOB")];

const fcaPrefGets = () =>
  api.GET.mock.calls.filter(([path, init]) => path === PREFS_PATH && init?.params?.path?.namespace === "fca");
const fcaPrefPuts = () =>
  api.PUT.mock.calls.filter(([path, init]) => path === PREFS_PATH && init?.params?.path?.namespace === "fca");

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  camera.fitBounds.mockReset();
  camera.home.mockReset();
  // Everything the map loads besides what each test seeds: an empty, successful answer.
  api.GET.mockResolvedValue({ data: undefined, error: undefined, response: { ok: true, status: 200 } });
  api.PUT.mockResolvedValue({ data: undefined, error: undefined, response: { ok: true, status: 200 } });
});

async function mount(qc: QueryClient, props: { persistArtccFilter?: boolean } = { persistArtccFilter: true }) {
  qc.setQueryData(["me"], CONTROLLER);
  qc.setQueryData(["fcas"], FCAS);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <FcaMapView persistKey="ops-fca" {...props} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="ARTCC"]');
  if (!select) throw new Error("ARTCC selector not rendered");
  return select;
}

const newClient = () =>
  new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
      mutations: { retry: false },
    },
  });

async function pick(select: HTMLSelectElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

const options = (select: HTMLSelectElement) => [...select.options].map((o) => o.value);

describe("FcaMapView ARTCC filter on /ops/fca (VATUSA/OIS#789)", () => {
  it("restores a saved ZDC, frames the map to it, and keeps it though ZDC has no FCAs today", async () => {
    const qc = newClient();
    qc.setQueryData(["preferences", "fca"], { artcc: "ZDC" });
    const select = await mount(qc);

    expect(select.value).toBe("ZDC");
    expect(options(select)).toEqual(["", "ZDC", "ZNY", "ZOB"]);
    expect(camera.fitBounds).toHaveBeenCalledTimes(1);
    expect(camera.fitBounds.mock.calls[0][0].length).toBeGreaterThan(0);
    // Showing it is not a reason to save it: the stored ZDC is left exactly as it is.
    expect(fcaPrefPuts()).toHaveLength(0);
  });

  it("frames the map once the saved ARTCC loads, and writes nothing while it loads", async () => {
    let resolve!: (v: unknown) => void;
    api.GET.mockImplementation((path: string, init?: { params?: { path?: { namespace?: string } } }) =>
      path === PREFS_PATH && init?.params?.path?.namespace === "fca"
        ? new Promise((r) => (resolve = r))
        : Promise.resolve({ data: undefined, error: undefined, response: { ok: true, status: 200 } }),
    );
    const qc = newClient();
    const select = await mount(qc);
    expect(select.value).toBe("");
    expect(camera.fitBounds).not.toHaveBeenCalled();

    await act(async () => resolve({ data: { artcc: "ZDC" }, response: { ok: true, status: 200 } }));
    // TanStack notifies outside React's scheduler, so keep flushing macrotasks until it re-renders.
    await vi.waitFor(async () => {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });
      expect(select.value).toBe("ZDC");
    });
    expect(camera.fitBounds).toHaveBeenCalledTimes(1);
    expect(fcaPrefPuts()).toHaveLength(0);
  });

  it("does not save a pick made before the saved ARTCC has loaded", async () => {
    api.GET.mockImplementation((path: string, init?: { params?: { path?: { namespace?: string } } }) =>
      path === PREFS_PATH && init?.params?.path?.namespace === "fca"
        ? new Promise(() => {})
        : Promise.resolve({ data: undefined, error: undefined, response: { ok: true, status: 200 } }),
    );
    const select = await mount(newClient());
    await pick(select, "ZNY");
    expect(select.value).toBe("ZNY");
    expect(fcaPrefGets()).toHaveLength(1);
    expect(fcaPrefPuts()).toHaveLength(0);
  });

  it("saves every pick, ALL ARTCCs included, and leaves the camera alone for ALL", async () => {
    const qc = newClient();
    qc.setQueryData(["preferences", "fca"], { artcc: "ZDC" });
    const select = await mount(qc);
    camera.fitBounds.mockReset();

    await pick(select, "ZNY");
    expect(fcaPrefPuts().map(([, init]) => init.body)).toEqual([{ artcc: "ZNY" }]);
    expect(camera.fitBounds).toHaveBeenCalledTimes(1);

    await pick(select, "");
    expect(select.value).toBe("");
    expect(fcaPrefPuts().map(([, init]) => init.body)).toEqual([{ artcc: "ZNY" }, { artcc: "" }]);
    expect(camera.fitBounds).toHaveBeenCalledTimes(1);
    expect(camera.home).not.toHaveBeenCalled();
  });

  it("restores a saved ALL ARTCCs without moving the map", async () => {
    const qc = newClient();
    qc.setQueryData(["preferences", "fca"], { artcc: "" });
    const select = await mount(qc);
    expect(select.value).toBe("");
    expect(camera.fitBounds).not.toHaveBeenCalled();
    expect(camera.home).not.toHaveBeenCalled();
  });

  // The namespace is an opaque blob the backend never checks, so a value that isn't a string is
  // treated as ALL rather than handed to the selector and the camera.
  it("treats a stored value that isn't a string as ALL ARTCCs", async () => {
    const qc = newClient();
    qc.setQueryData(["preferences", "fca"], { artcc: 42 });
    const select = await mount(qc);
    expect(select.value).toBe("");
    expect(options(select)).toEqual(["", "ZNY", "ZOB"]);
    expect(camera.fitBounds).not.toHaveBeenCalled();
  });

  // The advisories overview and the event builder keep today's reset-to-ALL (owner decision on #789).
  it("neither reads nor saves the filter, nor frames the map, where the page doesn't opt in", async () => {
    const qc = newClient();
    qc.setQueryData(["preferences", "fca"], { artcc: "ZDC" });
    const select = await mount(qc, {});
    expect(select.value).toBe("");

    await pick(select, "ZNY");
    expect(select.value).toBe("ZNY");
    expect(fcaPrefGets()).toHaveLength(0);
    expect(fcaPrefPuts()).toHaveLength(0);
    expect(camera.fitBounds).not.toHaveBeenCalled();
  });
});

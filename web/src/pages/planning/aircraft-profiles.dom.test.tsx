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
const put = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());
// The page puts "Add type" and "Add family" in the shell header; capture what it hands the header so
// a test can mount those actions (#818).
const header = vi.hoisted(() => ({current: null as null | {actions?: unknown}}));

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: put, DELETE: del}}));
vi.mock("@/components/shell/page-meta", () => ({
  usePageHeader: (o: {actions?: unknown}) => {
    header.current = o;
  },
}));

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
  put.mockReset();
  del.mockReset();
  header.current = null;
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

async function mount(seed: AircraftProfile[] | null, me: Me = ME) {
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
  qc.setQueryData(["me"], me);
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

// Coverage matrix (VATUSA/OIS#818, AC2: the admin page adds, lists, edits and deletes family rows)
//
// Requirement                                 | Test
// --------------------------------------------|-----------------------------------------------------------------------
// AC2 list: own heading, wake < family < type | lists a family row under "Engine families" between the wake and type sections
// AC2 list: label                             | labels a family row "Jet family" and a turboprop row "Turboprop family"
// AC2 list: name shown on a family row         | shows a family row's name beside its label
// AC2 add: PUT kind family + key              | adds a family through Add family: PUTs kind family and the chosen key
// AC2 add: no duplicate family offered        | offers only the engine families that have no row yet
// AC2 add: hidden when all families exist     | hides Add family once every engine family has a row, and keeps Add type
// AC2 edit: PUT with the row kind/key         | edits a family row: PUTs kind family and the row's key
// AC2 delete: DELETE kind family              | deletes a family row: DELETEs kind family and the row's key
// AC2 read-only: no controls                  | shows a read-only user no Add family, Edit or Delete controls
describe("Engine-family rows on the aircraft profiles page (#818 AC2)", () => {
  const row = (kind: string, key: string, desc_fpm: number): AircraftProfile => ({...B738, kind, key, name: "", desc_fpm});
  const DEFAULT = row("default", "", 1800);
  const WAKE_H = row("wake", "H", 2200);
  const JET = row("family", "jet", 2345);
  const TURBOPROP = row("family", "turboprop", 1500);
  const READ_ONLY = {...ME, permissions: {flow: {aircraft_profiles: ["read"]}}} as unknown as Me;

  /** Mount the header actions the page last handed `usePageHeader`, in their own root. */
  async function mountActions(qc: QueryClient) {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    await act(async () => {
      root.render(<QueryClientProvider client={qc}>{header.current?.actions as React.ReactNode}</QueryClientProvider>);
    });
    return host;
  }
  const button = (scope: ParentNode, text: string) => {
    const el = [...scope.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
    if (!el) throw new Error(`no ${text} button`);
    return el;
  };
  const section = (title: string) => {
    const h = [...document.querySelectorAll("h2")].find((e) => e.textContent?.startsWith(title));
    if (!h?.parentElement) throw new Error(`no ${title} section`);
    return h.parentElement;
  };
  /** Set a controlled input's value the way React sees a keystroke. */
  const typeInto = (el: HTMLInputElement | HTMLSelectElement, value: string) =>
    act(async () => {
      const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
      el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
    });
  const fieldInput = (label: string) => {
    const l = [...document.querySelectorAll("label")].find((e) => e.querySelector("span")?.textContent === label);
    const el = l?.querySelector("input, select");
    if (!el) throw new Error(`no ${label} field`);
    return el as HTMLInputElement | HTMLSelectElement;
  };

  it('lists a family row under "Engine families" between the wake and type sections', async () => {
    await mount([B738, JET, DEFAULT, WAKE_H]);

    const titles = [...document.querySelectorAll("h2")].map((h) => h.firstChild?.textContent);
    expect(titles).toEqual(["Default", "Wake classes", "Engine families", "Aircraft types"]);
    const rows = section("Engine families").querySelectorAll("tbody tr");
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain("Jet family");
  });

  it('labels a family row "Jet family" and a turboprop row "Turboprop family"', async () => {
    await mount([DEFAULT, JET, TURBOPROP]);

    const text = section("Engine families").textContent ?? "";
    expect(text).toContain("Jet family");
    expect(text).toContain("Turboprop family");
    expect(document.querySelector('button[aria-label="Delete Jet family"]')).not.toBeNull();
    expect(document.querySelector('button[aria-label="Delete Turboprop family"]')).not.toBeNull();
  });

  it("shows a family row's name beside its label", async () => {
    await mount([DEFAULT, {...JET, name: "Narrowbody jets"}]);

    const cell = section("Engine families").querySelector("tbody tr td");
    expect(cell?.textContent).toBe("Jet familyNarrowbody jets");
  });

  it("adds a family through Add family: PUTs kind family and the chosen key", async () => {
    put.mockResolvedValue({data: row("family", "turboprop", 1500), error: undefined, response: {status: 200}});
    const {qc} = await mount([DEFAULT, B738]);
    const actions = await mountActions(qc);

    await click(button(actions, "Add family"));
    expect(document.body.textContent).toContain("New engine family");
    await typeInto(fieldInput("Engine family"), "turboprop");
    await typeInto(fieldInput("Descent rate (fpm)"), "1500");
    await click(button(document, "Save"));
    await settle();

    expect(put).toHaveBeenCalledTimes(1);
    const [path, opts] = put.mock.calls[0];
    expect(path).toBe("/api/v1/flow/aircraft-profiles/{kind}/{key}");
    expect(opts.params.path).toEqual({kind: "family", key: "turboprop"});
    expect(opts.body.desc_fpm).toBe(1500);
    expect(document.body.textContent).not.toContain("New engine family");
  });

  it("adds a type through Add type: PUTs kind type and the typed key uppercased", async () => {
    put.mockResolvedValue({data: {...B738, key: "B38M"}, error: undefined, response: {status: 200}});
    const {qc} = await mount([DEFAULT, B738]);
    const actions = await mountActions(qc);

    await click(button(actions, "Add type"));
    expect(document.body.textContent).toContain("New aircraft type");
    expect(document.body.textContent).not.toContain("Engine family");
    await typeInto(fieldInput("ICAO type"), "b");
    expect(button(document, "Save").disabled).toBe(true);
    await typeInto(fieldInput("ICAO type"), "b38m");
    expect(button(document, "Save").disabled).toBe(false);
    await click(button(document, "Save"));
    await settle();

    expect(put).toHaveBeenCalledTimes(1);
    const [path, opts] = put.mock.calls[0];
    expect(path).toBe("/api/v1/flow/aircraft-profiles/{kind}/{key}");
    expect(opts.params.path).toEqual({kind: "type", key: "B38M"});
  });

  it("offers only the engine families that have no row yet", async () => {
    const options = () => [...(fieldInput("Engine family") as HTMLSelectElement).options].map((o) => o.value);

    const first = await mount([DEFAULT, B738]);
    await click(button(await mountActions(first.qc), "Add family"));
    expect(options()).toEqual(["jet", "turboprop", "piston"]);

    for (const {root, host} of roots.splice(0)) {
      act(() => root.unmount());
      host.remove();
    }
    const second = await mount([DEFAULT, JET, B738]);
    await click(button(await mountActions(second.qc), "Add family"));
    expect(options()).toEqual(["turboprop", "piston"]);
  });

  it("hides Add family once every engine family has a row, and keeps Add type", async () => {
    const PISTON = row("family", "piston", 600);
    const {qc} = await mount([DEFAULT, JET, TURBOPROP, PISTON, B738]);
    const actions = await mountActions(qc);

    expect([...actions.querySelectorAll("button")].map((b) => b.textContent?.trim())).toEqual(["Add type"]);
  });

  it("edits a family row: PUTs kind family and the row's key", async () => {
    put.mockResolvedValue({data: row("family", "jet", 2456), error: undefined, response: {status: 200}});
    await mount([DEFAULT, JET, B738]);

    await click(button(section("Engine families"), "Edit"));
    expect(document.body.textContent).toContain("Edit profile");
    await typeInto(fieldInput("Descent rate (fpm)"), "2456");
    await click(button(document, "Save"));

    expect(put).toHaveBeenCalledTimes(1);
    const [, opts] = put.mock.calls[0];
    expect(opts.params.path).toEqual({kind: "family", key: "jet"});
    expect(opts.body.desc_fpm).toBe(2456);
  });

  it("deletes a family row: DELETEs kind family and the row's key", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    await mount([DEFAULT, JET, B738]);
    const deleteJet = () => document.querySelector('button[aria-label="Delete Jet family"]')!;

    await click(deleteJet());
    expect(del).not.toHaveBeenCalled();
    expect(toasts().some((t) => t.includes('Delete "Jet family"?'))).toBe(true);
    await click(deleteJet());

    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/flow/aircraft-profiles/{kind}/{key}", {
      params: {path: {kind: "family", key: "jet"}},
    });
  });

  it("shows a read-only user no Add family, Edit or Delete controls", async () => {
    await mount([DEFAULT, JET, B738], READ_ONLY);

    expect(section("Engine families").textContent).toContain("Jet family");
    expect(header.current?.actions).toBeUndefined();
    expect([...document.querySelectorAll("button")].some((b) => b.textContent?.trim() === "Edit")).toBe(false);
    expect(document.querySelector('button[aria-label="Delete Jet family"]')).toBeNull();
  });

  it("shows an editor both add actions (positive control for the read-only test)", async () => {
    const {qc} = await mount([DEFAULT, B738]);
    const actions = await mountActions(qc);
    expect(button(actions, "Add family")).toBeTruthy();
    expect(button(actions, "Add type")).toBeTruthy();
  });
});

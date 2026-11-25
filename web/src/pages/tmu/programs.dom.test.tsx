// @vitest-environment jsdom
// Coverage matrix (#824, AC9 on TMU programs)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC9 failed refetch keeps table + armed button | keeps the table and an armed Remove when a background ... | QueryState gets `programs.isError` without
//                                              |                                                          | `&& !programs.data`
// AC9 control: error with no data              | shows the error state when the first load fails ...      | the gate drops `isError` altogether
//
// Coverage matrix (#1010, "Set program" keeps an existing program's restrictions)
//
// Requirement                                  | Test                                                     | Red when
// ---------------------------------------------|----------------------------------------------------------|------------------------------------------
// AC1/AC3 keeps gates, wake, types, jets_only  | keeps the existing program's gates, wake and type ...    | body restores `gates: []` etc. (any one of
//   aar/trail/until from the form              |                                                          | the four), or reads trail/until from existing
// AC1 form clears active_until                 | sends active_until null when the form leaves it blank ...| active_until falls back to the stored value
// AC1 MIT entry still keeps restrictions       | sends trail 0 with the form's MIT and still keeps ...    | `mitN > 0 ? 0 : trail` dropped, or MIT
//                                              |                                                          | branch rebuilds the body without existing
// AC1 lookup on the cleaned ICAO               | matches the existing program when the ICAO is typed ...  | lookup uses raw `icao`, not `cleanIcao`
// AC1 lookup keyed per airport                 | takes restrictions from the typed airport, not another...| lookup returns the first program / any match
// AC3 control: no program on the airport       | sends empty restrictions when the airport has no prog... | the no-match branch reuses a stale program
// AC2 list never loaded                        | sends nothing and warns when the program list has not ...| the `!programs.data` guard is removed
//   + positive control after load              |                                                          | (no warning toast)
// AC2 first load failed                        | sends nothing and warns when the program list failed ... | guard checks `isPending` only, not `!data`
//   + positive control after a refetch         |                                                          |
// AC2 failed refetch keeps cached data usable  | still sets a program from cached data after a backgr...  | guard keyed on `isError` instead of `!data`
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: put, DELETE: del}}));
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
  put.mockReset();
  vi.useRealTimers();
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

async function mount(seed: Program[] | null, me: Me = ME) {
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
  qc.setQueryData(["me"], me);
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

// The form renders only for a caller holding `tmu.program.update`, so this block mounts with its own
// Me (server_admin: false, real tree with `update`) instead of the #824 ME above. The PUT is read
// from the mocked `ois.PUT` (`/api/v1/tmu/programs/{icao}`, `{params: {path: {icao}}, body}`), and
// every expected body is a literal written here, never built from the seeded Program.
describe("Set program keeps an existing program's restrictions (#1010)", () => {
  const EDITOR = {
    ...ME,
    permissions: {tmu: {program: ["read", "update"]}},
  } as unknown as Me;

  // An existing program with every restriction the form does not show, and a trail and expiry the
  // form replaces.
  const KIAD_RESTRICTED: Program = {
    ...KIAD,
    trail: 10,
    gates: [
      {name: "CAVLR", trail: 7, mit: 0},
      {name: "GIBBZ", trail: 0, mit: 25},
    ],
    exclude_wake: ["L"],
    exclude_types: ["C172", "PA28"],
    jets_only: true,
    active_until: "2026-11-20T18:00:00.000Z",
  };
  const KIAD_KEPT = {
    gates: [
      {name: "CAVLR", trail: 7, mit: 0},
      {name: "GIBBZ", trail: 0, mit: 25},
    ],
    exclude_wake: ["L"],
    exclude_types: ["C172", "PA28"],
    jets_only: true,
  };
  const KDCA_RESTRICTED: Program = {
    ...KIAD,
    icao: "KDCA",
    gates: [{name: "TRUPS", trail: 5, mit: 0}],
    exclude_wake: ["M"],
    exclude_types: ["BE36"],
    jets_only: false,
  };
  const NOT_LOADED = "The program list hasn't loaded yet";

  // parseZulu resolves "DD/HHMMz" against the current month; pin the date so the expiry is a literal.
  function pinDate() {
    vi.useFakeTimers({toFake: ["Date"]});
    vi.setSystemTime(new Date("2026-11-05T00:00:00Z"));
  }

  function field(host: HTMLElement, label: string) {
    const el = [...host.querySelectorAll("label")]
      .find((l) => l.textContent?.trim().startsWith(label))
      ?.querySelector("input, select");
    if (!el) throw new Error(`no "${label}" field on the set-program form`);
    return el as HTMLInputElement | HTMLSelectElement;
  }
  /** React tracks the value through the prototype setter, so assigning `.value` alone is not seen. */
  async function fill(host: HTMLElement, label: string, value: string) {
    const el = field(host, label);
    const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    await act(async () => {
      Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
      el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
    });
  }
  function setProgramButton(host: HTMLElement) {
    const el = [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Set program");
    if (!el) throw new Error("no Set program button");
    return el;
  }
  const putAnswers = () =>
    put.mockImplementation((_path: string, {body}: {body: object}) =>
      Promise.resolve({data: {icao: "KIAD", ...body}, error: undefined, response: {status: 200}}),
    );
  const ok = (programs: Program[]) => () =>
    Promise.resolve({data: programs, error: undefined, response: {status: 200}});
  /** The body of the one PUT sent, after asserting there was exactly one and to which airport. */
  function sentBody(icao: string) {
    expect(put).toHaveBeenCalledTimes(1);
    const [path, init] = put.mock.calls[0];
    expect(path).toBe("/api/v1/tmu/programs/{icao}");
    expect(init.params).toEqual({path: {icao}});
    return init.body;
  }

  it("keeps the existing program's gates, wake and type exclusions and jets-only, taking AAR, trail and active-until from the form", async () => {
    pinDate();
    answerPrograms(ok([KIAD_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KIAD_RESTRICTED], EDITOR);

    await fill(host, "Airport", "KIAD");
    await fill(host, "AAR", "44");
    await fill(host, "Trail", "5");
    await fill(host, "Active until", "12/0400z");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({
      aar: 44,
      trail: 5,
      mit: 0,
      ...KIAD_KEPT,
      active_until: "2026-11-12T04:00:00.000Z",
    });
  });

  it("sends active_until null when the form leaves it blank over a program that had one", async () => {
    answerPrograms(ok([KIAD_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KIAD_RESTRICTED], EDITOR);

    await fill(host, "Airport", "KIAD");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
  });

  it("sends trail 0 with the form's MIT and still keeps the existing restrictions", async () => {
    answerPrograms(ok([KIAD_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KIAD_RESTRICTED], EDITOR);

    await fill(host, "Airport", "KIAD");
    await fill(host, "Trail", "5");
    await fill(host, "MIT", "20");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 20, ...KIAD_KEPT, active_until: null});
  });

  it("matches the existing program when the ICAO is typed in lowercase with stray characters", async () => {
    answerPrograms(ok([KIAD_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KIAD_RESTRICTED], EDITOR);

    await fill(host, "Airport", "kia-d");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
  });

  it("takes restrictions from the typed airport's program, not another airport's", async () => {
    answerPrograms(ok([KDCA_RESTRICTED, KIAD_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KDCA_RESTRICTED, KIAD_RESTRICTED], EDITOR);

    await fill(host, "Airport", "KIAD");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
  });

  it("sends empty restrictions and jets-only false when the airport has no program", async () => {
    answerPrograms(ok([KDCA_RESTRICTED]));
    putAnswers();
    const {host} = await mount([KDCA_RESTRICTED], EDITOR);

    await fill(host, "Airport", "KIAD");
    await fill(host, "AAR", "36");
    await click(setProgramButton(host));

    expect(sentBody("KIAD")).toEqual({
      aar: 36,
      trail: 0,
      mit: 0,
      gates: [],
      exclude_wake: [],
      exclude_types: [],
      jets_only: false,
      active_until: null,
    });
  });

  it("sends nothing and warns when the program list has not loaded yet", async () => {
    let resolveList: (v: unknown) => void = () => {};
    answerPrograms(() => new Promise((r) => (resolveList = r)));
    putAnswers();
    const {host, qc} = await mount(null, EDITOR);
    expect(qc.getQueryState(["tmu-programs"])?.status).toBe("pending");

    await fill(host, "Airport", "KIAD");
    await click(setProgramButton(host));
    expect(put).not.toHaveBeenCalled();
    expect(toasts().some((t) => t.includes(NOT_LOADED))).toBe(true);

    // Positive control: once the list arrives, the same submit sends and keeps the restrictions.
    await act(async () => resolveList({data: [KIAD_RESTRICTED], error: undefined, response: {status: 200}}));
    await settle();
    await click(setProgramButton(host));
    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
  });

  it("sends nothing and warns when the program list failed to load", async () => {
    answerPrograms(() => Promise.resolve({data: undefined, error: {error: "unavailable"}, response: {status: 503}}));
    putAnswers();
    const {host, qc} = await mount(null, EDITOR);
    await settle();
    expect(qc.getQueryState(["tmu-programs"])?.status).toBe("error");

    await fill(host, "Airport", "KIAD");
    await click(setProgramButton(host));
    expect(put).not.toHaveBeenCalled();
    expect(toasts().some((t) => t.includes(NOT_LOADED))).toBe(true);

    // Positive control: a refetch that succeeds lets the same submit through.
    answerPrograms(ok([KIAD_RESTRICTED]));
    await act(async () => {
      await qc.refetchQueries({queryKey: ["tmu-programs"]});
    });
    await settle();
    await click(setProgramButton(host));
    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
  });

  it("still sets a program from cached data after a background refetch fails", async () => {
    answerPrograms(() => Promise.resolve({data: undefined, error: {error: "unavailable"}, response: {status: 503}}));
    putAnswers();
    const {host, qc} = await mount([KIAD_RESTRICTED], EDITOR);
    await act(async () => {
      await qc.refetchQueries({queryKey: ["tmu-programs"]});
    });
    await settle();
    // The refetch really failed and the cache kept the program.
    expect(qc.getQueryState(["tmu-programs"])?.status).toBe("error");
    expect(qc.getQueryData<Program[]>(["tmu-programs"])).toHaveLength(1);

    await fill(host, "Airport", "KIAD");
    await click(setProgramButton(host));
    expect(sentBody("KIAD")).toEqual({aar: 30, trail: 0, mit: 0, ...KIAD_KEPT, active_until: null});
    expect(toasts().some((t) => t.includes(NOT_LOADED))).toBe(false);
  });
});

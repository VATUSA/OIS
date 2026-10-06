// @vitest-environment jsdom
//
// VATUSA/OIS#746. The live-operation routes — reorder, mark/clear a release, swap — answer 404 once an
// event FCA is planned or archived, and for an FCA or flight that has otherwise gone. The hooks must
// say so neutrally rather than as a failure, refetch whatever could still show the stale row, and
// leave no optimistic reorder behind. Any other refusal keeps its own message.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const api = vi.hoisted(() => ({ POST: vi.fn(), PUT: vi.fn(), DELETE: vi.fn() }));
vi.mock("@/lib/api", () => ({ ois: api, API_BASE: "", DOCS_URL: "" }));

import {type FcaFlight, useClearRelease, useMarkRelease, useReorderFca, useSwapReleases} from "./fca";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
});

const ID = "f-1";
const answer = (status: number) => ({
  data: undefined,
  error: { error: status === 404 ? "not_found" : "internal" },
  response: { status, ok: false },
});

/** Every query a vanished FCA or flight could still be showing in. */
const STALE_KEYS = [["fcas"], ["fca-traffic", ID], ["idst"], ["departures"]];

const flight = (callsign: string, seq: number) => ({ callsign, seq }) as unknown as FcaFlight;
const TRAFFIC = [flight("AAL1", 1), flight("DAL2", 2)];

type Hooks = {
  mark: ReturnType<typeof useMarkRelease>;
  clear: ReturnType<typeof useClearRelease>;
  swap: ReturnType<typeof useSwapReleases>;
  reorder: ReturnType<typeof useReorderFca>;
};

/** Mounts the four hooks against a real cache seeded with every stale-able query. */
function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  for (const key of STALE_KEYS) qc.setQueryData(key, key[0] === "fca-traffic" ? TRAFFIC : []);
  const hooks = {} as Hooks;
  function Harness() {
    hooks.mark = useMarkRelease(ID);
    hooks.clear = useClearRelease(ID);
    hooks.swap = useSwapReleases(ID);
    hooks.reorder = useReorderFca(ID);
    return null;
  }
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <Harness />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return { qc, hooks, host };
}

async function run(fire: () => Promise<unknown>) {
  await act(async () => {
    await fire().catch(() => {});
  });
}

const stale = (qc: QueryClient) =>
  STALE_KEYS.filter((key) => qc.getQueryState(key)?.isInvalidated).map((key) => key[0]);

const CASES: [string, keyof typeof api, (h: Hooks) => Promise<unknown>, string][] = [
  ["mark a release", "POST", (h) => h.mark.mutateAsync({ callsign: "AAL1" }), "Couldn’t issue the release"],
  ["clear a release", "DELETE", (h) => h.clear.mutateAsync("DAL2"), "Couldn’t clear the release"],
  ["swap releases", "POST", (h) => h.swap.mutateAsync({ a: "AAL1", b: "DAL2" }), "Couldn’t swap the releases"],
  ["reorder", "PUT", (h) => h.reorder.mutateAsync(["DAL2", "AAL1"]), "Couldn’t reorder"],
];

describe("live FCA operations on a 404 (VATUSA/OIS#746)", () => {
  it.each(CASES)("%s: says it's no longer available and refetches the stale views", async (_l, verb, fire, failure) => {
    api[verb].mockResolvedValue(answer(404));
    const { qc, hooks, host } = mount();
    await run(() => fire(hooks));
    expect(host.textContent).toContain("No longer available");
    expect(host.textContent).not.toContain(failure);
    expect(stale(qc)).toEqual(["fcas", "fca-traffic", "idst", "departures"]);
  });

  it.each(CASES)("%s: any other refusal keeps its own message", async (_l, verb, fire, failure) => {
    api[verb].mockResolvedValue(answer(500));
    const { qc, hooks, host } = mount();
    await run(() => fire(hooks));
    expect(host.textContent).toContain(failure);
    expect(host.textContent).not.toContain("No longer available");
    expect(stale(qc)).not.toContain("idst");
  });

  it("rolls the optimistic reorder back rather than leaving it stuck", async () => {
    api.PUT.mockResolvedValue(answer(404));
    const { qc, hooks } = mount();
    await run(() => hooks.reorder.mutateAsync(["DAL2", "AAL1"]));
    expect(qc.getQueryData(["fca-traffic", ID])).toEqual(TRAFFIC);
  });
});

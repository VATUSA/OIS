// @vitest-environment jsdom
//
// VATUSA/OIS#1009: a preferences save fails on any non-OK response. openapi-fetch leaves `error`
// undefined for a failure with no body, so a check on `error` alone counted that failure as saved.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const api = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn() }));
vi.mock("@/lib/api", () => ({ ois: api, API_BASE: "", DOCS_URL: "" }));

import {useSavePreferences} from "./preferences";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

type Scope = { airports: string[] };
const STORED: Scope = { airports: ["KIAD"] };
const NEW: Scope = { airports: ["KDCA"] };

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});
beforeEach(() => {
  api.PUT.mockReset();
});

/** Mounts the hook against a cache holding STORED; returns the client and a getter for the mutation. */
function mountSave() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } },
  });
  qc.setQueryData(["preferences", "idst"], STORED);
  let save!: ReturnType<typeof useSavePreferences<Scope>>;
  function Probe() {
    save = useSavePreferences<Scope>("idst");
    return null;
  }
  const host = document.createElement("div");
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <Probe />
      </QueryClientProvider>,
    ),
  );
  return { qc, save: () => save };
}

async function outcome(run: () => Promise<unknown>): Promise<"resolved" | "rejected"> {
  let result: "resolved" | "rejected" = "resolved";
  await act(async () => {
    await run().catch(() => {
      result = "rejected";
    });
  });
  return result;
}

describe("useSavePreferences", () => {
  it("resolves an ok PUT and primes the preferences cache with the saved value", async () => {
    api.PUT.mockResolvedValue({ data: undefined, error: undefined, response: { ok: true, status: 200 } });
    const { qc, save } = mountSave();
    expect(await outcome(() => save().mutateAsync(NEW))).toBe("resolved");
    expect(qc.getQueryData(["preferences", "idst"])).toEqual(NEW);
  });

  it("rejects a PUT that fails with no error body and leaves the cache unchanged", async () => {
    api.PUT.mockResolvedValue({ data: undefined, error: undefined, response: { ok: false, status: 502 } });
    const { qc, save } = mountSave();
    expect(await outcome(() => save().mutateAsync(NEW))).toBe("rejected");
    expect(qc.getQueryData(["preferences", "idst"])).toEqual(STORED);
  });

  it("rejects a PUT that fails with an error body and leaves the cache unchanged", async () => {
    api.PUT.mockResolvedValue({ data: undefined, error: { error: "internal" }, response: { ok: false, status: 500 } });
    const { qc, save } = mountSave();
    expect(await outcome(() => save().mutateAsync(NEW))).toBe("rejected");
    expect(qc.getQueryData(["preferences", "idst"])).toEqual(STORED);
  });
});

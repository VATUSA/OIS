// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {RestrictionAlerts} from "./restriction-alerts";

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
  document.body.innerHTML = "";
});

/**
 * A ZDC controller who runs traffic management. Not `server_admin` — that short-circuits
 * `hasPermission` and would let the component render for the wrong reason, masking the scoping.
 */
const zdcController = {
  id: "u1",
  cid: 1,
  email: "a@b.c",
  display_name: "Tester",
  rating: null,
  server_admin: false,
  role_names: [],
  tmu_national: false,
  permissions: { tmu: { program: ["read"] } },
  vatusa: { home_facility: "ZDC", visits: [] },
} as never;

const nationalController = { ...(zdcController as object), tmu_national: true } as never;

const groundStop = (id: string, airport: string, artcc: string | null) => ({
  id,
  airport,
  artcc,
  scope: "",
  until: null,
  status: "published",
  published_at: null,
  updated_at: "2026-09-27T00:00:00Z",
  updated_by: null,
});

/**
 * Mounts the real component against the real query cache. The lists are seeded rather than fetched —
 * the generated client captures `fetch` at module load, so a stub here would never be seen.
 *
 * `known` is null until the first full load, so the set present at mount is captured silently. To get
 * an alert at all, mount with the lists already settled and *then* publish the new restrictions.
 */
async function mountWith(me: unknown) {
  // No network at all: the seeded lists must not refetch on mount. A failing background fetch would
  // otherwise re-render after the act block and make every assertion race it.
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchInterval: false,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        staleTime: Infinity,
      },
    },
  });
  qc.setQueryData(["me"], me);
  qc.setQueryData(["ground-stops"], []);
  qc.setQueryData(["gdps"], []);
  qc.setQueryData(["tmis"], []);
  qc.setQueryData(["tmu-programs"], []);

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <RestrictionAlerts />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });

  // The macrotask matters: TanStack's notify manager batches cache notifications outside React's
  // scheduler, so a plain `await act(async () => setQueryData(...))` returns before the re-render.
  const publish = async (stops: unknown[]) => {
    await act(async () => {
      qc.setQueryData(["ground-stops"], stops);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };
  const alerted = () =>
    [...document.querySelectorAll('[role="alert"]')].map((el) => el.textContent ?? "");
  return { publish, alerted };
}

describe("RestrictionAlerts scoping (VATUSA/OIS#405)", () => {
  it("alerts a ZDC controller to a ZDC ground stop, and not to a ZLA one", async () => {
    const { publish, alerted } = await mountWith(zdcController);
    await publish([groundStop("gs1", "KDCA", "ZDC"), groundStop("gs2", "KLAX", "ZLA")]);

    const text = alerted().join(" | ");
    expect(text).toContain("KDCA");
    expect(text).not.toContain("KLAX");
    expect(alerted()).toHaveLength(1);
  });

  it("alerts a national TMU reader to both", async () => {
    const { publish, alerted } = await mountWith(nationalController);
    await publish([groundStop("gs1", "KDCA", "ZDC"), groundStop("gs2", "KLAX", "ZLA")]);

    const text = alerted().join(" | ");
    expect(text).toContain("KDCA");
    expect(text).toContain("KLAX");
    expect(alerted()).toHaveLength(2);
  });

  it("stays quiet about a restriction whose ARTCC the facility map could not resolve", async () => {
    const { publish, alerted } = await mountWith(zdcController);
    await publish([groundStop("gs1", "KXXX", null)]);

    expect(alerted()).toHaveLength(0);
  });
});

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
  const set = async (key: string, value: unknown) => {
    await act(async () => {
      qc.setQueryData([key], value);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };
  const publish = (stops: unknown[]) => set("ground-stops", stops);
  const publishTo = (key: "gdps" | "tmis" | "tmu-programs", rows: unknown[]) => set(key, rows);
  const setMe = (next: unknown) => set("me", next);
  const alerted = () =>
    [...document.querySelectorAll('[role="alert"]')].map((el) => el.textContent ?? "");
  return { publish, publishTo, setMe, alerted };
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

  // Fail open: scoping narrows an audience that used to be everyone, and a ground stop the map
  // couldn't place (a 3-letter id, a lowercase event TMI) went to no one outside national TMU.
  it("alerts about a restriction whose ARTCC the facility map could not resolve", async () => {
    const { publish, alerted } = await mountWith(zdcController);
    await publish([groundStop("gs1", "KXXX", null)]);

    expect(alerted()).toHaveLength(1);
  });
});

const gdp = (id: string, airport: string, artcc: string | null) => ({
  id, airport, artcc, status: "published", aar: 40, start_time: "2026-09-27T12:00:00Z",
  end_time: "2026-09-27T14:00:00Z", scope: "",
});
const tmi = (id: string, requesting_artcc: string | null, providing_artcc: string | null) => ({
  id, status: "published", requesting: requesting_artcc ?? "XXX", providing: providing_artcc ?? "XXX",
  requesting_artcc, providing_artcc, restriction: "20 MIT", decoded: "",
});
const program = (icao: string, artcc: string | null) => ({icao, artcc, aar: 40, trail: 0, mit: 0, jets_only: false});

// Each list filters on its own field; only ground stops were pinned (VATUSA/OIS#405 review).
describe("RestrictionAlerts scoping, per list (VATUSA/OIS#405 review)", () => {
  it("scopes GDPs", async () => {
    const {publishTo, alerted} = await mountWith(zdcController);
    await publishTo("gdps", [gdp("g1", "KIAD", "ZDC"), gdp("g2", "KSFO", "ZOA")]);
    expect(alerted()).toHaveLength(1);
    expect(alerted()[0]).toContain("KIAD");
  });

  it("scopes metering programs", async () => {
    const {publishTo, alerted} = await mountWith(zdcController);
    await publishTo("tmu-programs", [program("KDCA", "ZDC"), program("KORD", "ZAU")]);
    expect(alerted()).toHaveLength(1);
    expect(alerted()[0]).toContain("KDCA");
  });

  it("alerts a centre to a TMI it is providing, not only one it requested", async () => {
    const {publishTo, alerted} = await mountWith(zdcController);
    await publishTo("tmis", [tmi("t1", "ZNY", "ZDC"), tmi("t2", "ZNY", "ZBW")]);
    expect(alerted()).toHaveLength(1);
  });
});

describe("RestrictionAlerts when the scope changes (VATUSA/OIS#405 review)", () => {
  // Diffed against the old scope, every restriction already running in the new one announced
  // itself as just initiated — someone made national was alerted to the whole country at once.
  it("does not replay running restrictions as new when the scope widens", async () => {
    const {publish, setMe, alerted} = await mountWith(zdcController);
    await publish([groundStop("gs1", "KDCA", "ZDC")]);
    expect(alerted()).toHaveLength(1);

    await publish([groundStop("gs1", "KDCA", "ZDC"), groundStop("gs2", "KLAX", "ZLA")]);
    await setMe(nationalController); // KLAX, already running, is now in scope

    expect(alerted()).toHaveLength(1);
    expect(alerted().join(" ")).not.toContain("KLAX");
  });

  it("still alerts to a restriction initiated after the scope changed", async () => {
    const {publish, setMe, alerted} = await mountWith(zdcController);
    await setMe(nationalController);
    await publish([groundStop("gs3", "KSEA", "ZSE")]);

    expect(alerted().join(" ")).toContain("KSEA");
  });
});

// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {AtcWidgetView} from "./atc-widget";
import type {AtcWidget} from "./types";

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

const pos = (callsign: string, kind: string) => ({
  callsign,
  kind,
  frequency: "123.450",
  logon_time: "2026-09-30T12:00:00Z",
  name: "A Controller",
  atis_code: null,
});

/**
 * Two centers, one TRACON and two airports, deliberately spread across facilities: ZDC owns only
 * KDCA, so anything ZNY/KJFK proves the view is not filtering to one facility.
 */
const board = {
  as_of: "2026-09-30T14:32:00Z",
  centers: [
    { id: "ZDC", positions: [pos("ZDC_CTR", "CTR")] },
    { id: "ZNY", positions: [pos("ZNY_CTR", "CTR")] },
    // An empty facility must not render a stray heading.
    { id: "ZOB", positions: [] },
  ],
  tracons: [{ id: "PCT", positions: [pos("PCT_APP", "APP")], rings: [], circle: null, label: null, name: null }],
  airports: [
    { icao: "KJFK", lat: 0, lon: 0, positions: [pos("JFK_TWR", "TWR")] },
    { icao: "KDCA", lat: 0, lon: 0, positions: [pos("DCA_TWR", "TWR")] },
  ],
};

const directory = [
  { id: "ZDC", kind: "artcc", name: "Washington Center", airports: ["KDCA"] },
  { id: "ZNY", kind: "artcc", name: "New York Center", airports: ["KJFK"] },
];

/** Mounts the real widget against a seeded cache — the generated client captures `fetch` at module
 * load, so a stub here would never be seen. */
async function mount(widget: AtcWidget) {
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
  qc.setQueryData(["flow-atc"], board);
  qc.setQueryData(["flow-facilities"], directory);

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <AtcWidgetView widget={widget} />
      </QueryClientProvider>,
    );
  });
  return host.textContent ?? "";
}

const nationalWidget: AtcWidget = { id: "w1", kind: "atc", facility: { kind: "national" } };
const zdcWidget: AtcWidget = { id: "w2", kind: "atc", facility: { kind: "artcc", id: "ZDC" } };

describe("AtcWidgetView scope (VATUSA/OIS#474)", () => {
  it("shows every facility's positions when scoped to the NAS", async () => {
    const text = await mount(nationalWidget);

    // More than one ARTCC is the point: a view that filtered to one facility could never show both.
    expect(text).toContain("ZDC_CTR");
    expect(text).toContain("ZNY_CTR");
    expect(text).toContain("PCT_APP");
    expect(text).toContain("JFK_TWR");
    expect(text).toContain("DCA_TWR");
  });

  it("omits a facility with nobody online rather than rendering an empty heading", async () => {
    const text = await mount(nationalWidget);
    expect(text).not.toContain("ZOB");
  });

  it("still narrows to one facility and its member airports when scoped to an ARTCC", async () => {
    const text = await mount(zdcWidget);

    expect(text).toContain("ZDC_CTR");
    expect(text).toContain("DCA_TWR"); // KDCA is a ZDC airport
    expect(text).not.toContain("ZNY_CTR");
    expect(text).not.toContain("JFK_TWR"); // KJFK is not
    expect(text).not.toContain("PCT_APP");
  });

  it("says so when the whole NAS is quiet", async () => {
    const empty: AtcWidget = { id: "w3", kind: "atc", facility: { kind: "national" } };
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false, refetchInterval: false, staleTime: Infinity } },
    });
    qc.setQueryData(["flow-atc"], { ...board, centers: [], tracons: [], airports: [] });
    qc.setQueryData(["flow-facilities"], directory);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({ root, host });
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <AtcWidgetView widget={empty} />
        </QueryClientProvider>,
      );
    });
    expect(host.textContent ?? "").toContain("No online ATC anywhere in the NAS.");
  });
});

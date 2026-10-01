// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {DATA_SOURCES_BY_ID, type Row} from "./sources";

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

const demandRow = (icao: string, exceedance: number) => ({
  icao,
  demand_60min: 40 + exceedance,
  aar: 40,
  exceedance,
  aar_source: "program",
  inbound: 12,
  airborne: 8,
  ground: 4,
});

/**
 * Renders one source's `useRows` and reports both the rows and the query cache.
 *
 * The cache is the assertion that matters: a national source must resolve to ONE request no matter
 * how many airports come back. The per-airport sources go through `useQueries` over an ICAO list
 * (`lib/historical.ts`), so a regression that routed a national source down that path would show up
 * here as more than one cache entry.
 */
async function renderSource(sourceId: string, seed: [string[], unknown][]) {
  const source = DATA_SOURCES_BY_ID[sourceId];
  if (!source) throw new Error(`no such source: ${sourceId}`);

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
  for (const [key, value] of seed) qc.setQueryData(key, value);

  let rows: Row[] = [];
  function Probe() {
    rows = source!.useRows({}).rows;
    return null;
  }

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <Probe />
      </QueryClientProvider>,
    );
  });
  return { rows, queries: qc.getQueryCache().getAll() };
}

describe("national dashboard sources (VATUSA/OIS#475)", () => {
  it("fetches the whole NAS demand ranking in exactly one request", async () => {
    const { rows, queries } = await renderSource("nas-demand", [
      [["tmu-demand"], [demandRow("KJFK", 12), demandRow("KDCA", -3), demandRow("KLAX", 5)]],
    ]);

    // Three airports, one request — not one request per airport.
    expect(rows).toHaveLength(3);
    expect(queries).toHaveLength(1);
    expect(queries[0]?.queryKey).toEqual(["tmu-demand"]);
  });

  it("takes no airport parameters, so a national scope can't become an airport list", async () => {
    const source = DATA_SOURCES_BY_ID["nas-demand"];
    expect(source?.needsIcao).toBe(false);
    expect(source?.category).toBe("national");

    // Passing airports must not change the request shape — the source ignores them entirely.
    const { queries } = await renderSource("nas-demand", [[["tmu-demand"], []]]);
    expect(queries).toHaveLength(1);
  });

  it("ranks FCA pressure from the existing counts aggregate, not a new one", async () => {
    const { rows, queries } = await renderSource("nas-fca-pressure", [
      [["fcas"], [{ id: "f1", name: "ZDC ARRIVALS", artcc: "ZDC", rate: 30, mode: "mit" }]],
      [["fca-counts"], { f1: 17 }],
    ]);

    expect(rows).toHaveLength(1);
    expect(rows[0]?.count).toBe(17);
    // Exactly the two existing national queries — no third, airport-shaped one.
    expect(queries.map((q) => q.queryKey[0]).sort()).toEqual(["fca-counts", "fcas"]);
  });
});

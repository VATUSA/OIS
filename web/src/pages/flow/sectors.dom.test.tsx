// @vitest-environment jsdom
//
// VATUSA/OIS#821 AC5: the admin Sectors map draws the loaded sector volumes for the selected strata.
// It drew nothing on `next`: the page sets no `getCursor`, and `MapCanvas` handed deck an explicit
// `undefined` that replaced deck's default, so deck threw on its first frame. This mounts the page
// itself, so it covers both halves a person sees: the loaded volumes reach deck as the sector
// layer, filtered to the strata toggles, and deck runs with a cursor function. `DeckGL` cannot mount
// in jsdom (no WebGL), so it is faked at the module boundary and its props are merged over deck's
// real defaults the way deck merges them.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {Deck} from "@deck.gl/core";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {SectorVolume} from "@/lib/sectors";

const deck = vi.hoisted(() => ({current: null as null | Record<string, unknown>}));

vi.mock("@deck.gl/react", () => ({
  default: (props: Record<string, unknown>) => {
    deck.current = props;
    return null;
  },
}));
vi.mock("react-map-gl/maplibre", () => ({Map: () => null}));
vi.mock("maplibre-gl/dist/maplibre-gl.css", () => ({}));
vi.mock("@ois/ui", async (orig) => ({
  ...(await orig<typeof import("@ois/ui")>()),
  useTheme: () => ({resolvedTheme: "dark"}),
}));
vi.mock("@/components/map/hooks/useWebglAvailable", () => ({
  useWebglAvailable: () => ({ok: true, retry: () => {}}),
}));
vi.mock("@/components/map/lib/aeroway", () => ({ensureAeroway: () => {}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {SectorsPage} from "./sectors";

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
  deck.current = null;
});

const square = [
  [38, -77],
  [38, -76],
  [39, -76],
  [39, -77],
  [38, -77],
];
const volume = (artcc: string, volume_id: string, tier: string): SectorVolume => ({
  artcc,
  sector_id: volume_id.slice(0, 2),
  volume_id,
  name: null,
  tier,
  base_alt_ft: 0,
  top_alt_ft: 60_000,
  rings: [square],
});

/** Mounts the page against a seeded cache: the loaded volumes, and a settings blob with no strata. */
async function mount() {
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
  qc.setQueryData(
    ["airspace-sectors"],
    [
      volume("ZDC", "01001", "high"),
      volume("ZNY", "03001", "high"),
      volume("ZDC", "02001", "low"),
      volume("ZNY", "07001", "approach"),
    ],
  );
  qc.setQueryData(["preferences", "settings"], {});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <SectorsPage />
      </QueryClientProvider>,
    );
  });
  return host;
}

/** What deck would draw: the sector layer's volumes, by id. */
function drawn(): string[] {
  expect(deck.current, "the page never rendered the map").not.toBeNull();
  const layers = deck.current!.layers as {id: string; props: {data: {volume: SectorVolume}[]}}[];
  const sectors = layers.find((l) => l.id === "airspace-sectors");
  expect(sectors, "no airspace-sectors layer handed to deck").toBeDefined();
  return sectors!.props.data.map((d) => `${d.volume.artcc} ${d.volume.volume_id}`).sort();
}

/** Click a stratum toggle. Its setting is written to the query cache at once, and TanStack
 * notifies outside React's scheduler, so the act flushes a macrotask before the layers are read.
 * The save behind it fails against the test's disabled network and writes nothing back. */
async function toggle(host: HTMLElement, label: string) {
  const button = [...host.querySelectorAll("button")].find((b) => b.textContent === label);
  expect(button, `no ${label} toggle`).toBeDefined();
  await act(async () => {
    button!.click();
    await new Promise((r) => setTimeout(r, 0));
  });
}

describe("SectorsPage", () => {
  it("hands deck the loaded volumes of the selected strata, with a cursor it can call", async () => {
    const host = await mount();

    // High is the default stratum.
    expect(drawn()).toEqual(["ZDC 01001", "ZNY 03001"]);
    const getCursor = ({...Deck.defaultProps, ...deck.current} as Record<string, unknown>).getCursor;
    expect(getCursor).toBeTypeOf("function");

    await toggle(host, "Low");
    expect(drawn()).toEqual(["ZDC 01001", "ZDC 02001", "ZNY 03001"]);

    await toggle(host, "High");
    expect(drawn()).toEqual(["ZDC 02001"]);
  });
});

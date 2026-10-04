import {describe, expect, it} from "vitest";

import type {SectorVolume} from "@/lib/sectors";
import {readMapPalette} from "../lib/colors";
import {buildSectorLayer} from "./sectors";

const volume = (tier: string, rings: number[][][], volume_id = "03201"): SectorVolume => ({
  artcc: "ZDC",
  sector_id: "32",
  volume_id,
  name: null,
  tier,
  base_alt_ft: 24_000,
  top_alt_ft: 35_000,
  rings,
});

/** A plain square, `[lat, lon]`. */
const square = [
  [38, -77],
  [38, -76],
  [39, -76],
  [39, -77],
  [38, -77],
];

type Datum = { volume: SectorVolume; polygon: [number, number][] };
const data = (vols: SectorVolume[], strata: string[]) =>
  buildSectorLayer(vols, strata, readMapPalette()).props.data as Datum[];

describe("buildSectorLayer", () => {
  /** #481's shape: two lobes joined at a revisited vertex. Drawn as one polygon it fills a wedge
   * across the map; sanitized, it is two separate polygons and neither revisits a vertex. */
  it("splits a bridged ring rather than drawing it as a wedge", () => {
    const bridged = [
      [38, -77],
      [38, -76],
      [39, -76],
      [38, -77],
      [35, -80],
      [35, -79],
      [36, -79],
      [38, -77],
    ];
    const out = data([volume("high", [bridged])], ["high"]);
    expect(out).toHaveLength(2);
    for (const { polygon } of out) {
      const keys = polygon.map(([lon, lat]) => `${lat},${lon}`);
      expect(new Set(keys).size).toBe(keys.length);
    }
  });

  it("draws in deck's [lon, lat] order", () => {
    const [d] = data([volume("high", [square])], ["high"]);
    expect(d.polygon[0]).toEqual([-77, 38]);
  });

  it("draws only the selected strata, so stacked sectors separate", () => {
    const vols = [volume("low", [square], "a"), volume("high", [square], "b"), volume("ultra_high", [square], "c")];
    expect(data(vols, ["high"]).map((d) => d.volume.volume_id)).toEqual(["b"]);
    expect(data(vols, ["low", "ultra_high"]).map((d) => d.volume.volume_id)).toEqual(["a", "c"]);
    expect(data(vols, [])).toEqual([]);
  });

  it("is pickable, for the hover card", () => {
    expect(buildSectorLayer([], ["high"], readMapPalette()).props.pickable).toBe(true);
  });
});

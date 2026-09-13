import {describe, expect, it} from "vitest";

import {buildMatchedLayers, type MatchedFlight} from "./matched";

const flight = (over: Partial<MatchedFlight>): MatchedFlight => ({
  callsign: "AAL1",
  aircraft_type: "B738",
  seq: 1,
  heading: 90,
  lat: 40,
  lon: -80,
  cross_lat: 41,
  cross_lon: -79,
  path: [],
  dep: "KDFW",
  arr: "KORD",
  altitude: 35000,
  groundspeed: 420,
  distance_nm: 50,
  ...over,
});

function byId(layers: ReturnType<typeof buildMatchedLayers>, id: string) {
  return layers.find((l) => l.id === id);
}

describe("buildMatchedLayers", () => {
  it("excludes a flight with no live position (0,0) from the glyph and badge layers", () => {
    const layers = buildMatchedLayers([flight({ lat: 0, lon: 0 })], "#ff0000", "triangle");
    const glyphs = byId(layers, "matched");
    const badges = byId(layers, "matched-seq");
    expect((glyphs!.props.data as unknown[]).length).toBe(0);
    expect((badges!.props.data as unknown[]).length).toBe(0);
  });

  it("excludes a flight with no live position from the trail layer too (same filtered set)", () => {
    const layers = buildMatchedLayers([flight({ lat: 0, lon: 0 })], "#ff0000", "triangle");
    const trails = byId(layers, "matched-trails");
    expect((trails!.props.data as unknown[]).length).toBe(0);
  });

  it("still draws the crossing-point dot for a flight with no live position (unfiltered layer)", () => {
    const layers = buildMatchedLayers([flight({ lat: 0, lon: 0 })], "#ff0000", "triangle");
    const crossDots = byId(layers, "matched-cross");
    expect((crossDots!.props.data as unknown[]).length).toBe(1);
  });

  it("falls back to a straight line to the crossing point when no path is given", () => {
    const layers = byId(
      buildMatchedLayers([flight({ path: [] })], "#ff0000", "triangle"),
      "matched-trails",
    )!;
    const getPath = (layers.props as unknown as { getPath: (f: MatchedFlight) => [number, number][] })
      .getPath;
    expect(getPath(flight({ path: [] }))).toEqual([
      [-80, 40],
      [-79, 41],
    ]);
  });

  it("uses the real remaining-route path (converted to deck order) when one is given", () => {
    const layers = byId(
      buildMatchedLayers(
        [flight({ path: [[40, -80], [40.5, -79.5]] })],
        "#ff0000",
        "triangle",
      ),
      "matched-trails",
    )!;
    const getPath = (layers.props as unknown as { getPath: (f: MatchedFlight) => [number, number][] })
      .getPath;
    const f = flight({ path: [[40, -80], [40.5, -79.5]] });
    expect(getPath(f)).toEqual([
      [-80, 40],
      [-79.5, 40.5],
    ]);
  });

  it("appends the group-id suffix to every layer's id (overview mode)", () => {
    const layers = buildMatchedLayers([flight({})], "#ff0000", "triangle", 1, "-fca1");
    expect(byId(layers, "matched")).toBeUndefined();
    expect(byId(layers, "matched-fca1")).toBeDefined();
  });
});

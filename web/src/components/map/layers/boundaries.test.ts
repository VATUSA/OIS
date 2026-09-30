import {describe, expect, it} from "vitest";

import {buildBoundaryLayer} from "./boundaries";

/**
 * VATUSA/OIS#481 — `buildBoundaryLayer` had no test file at all, and it fills when `emphasis` is set,
 * so a bridged ring wedges here exactly as it does in `atc-centers`. The facility map reaches this
 * path for the selected facility whether or not anyone is online.
 */
describe("buildBoundaryLayer", () => {
  const bridged: GeoJSON.FeatureCollection = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: {id: "ZZZ"},
        geometry: {
          type: "Polygon",
          coordinates: [[[-80, 40], [-80, 41], [-79, 41], [-80, 40], [-75, 45], [-75, 46], [-74, 46], [-80, 40]]],
        },
      } as GeoJSON.Feature,
    ],
  };

  it("splits a bridged ring before it can be filled", () => {
    const data = buildBoundaryLayer(bridged, undefined, true).props.data as GeoJSON.FeatureCollection;
    const g = data.features[0].geometry as GeoJSON.MultiPolygon;
    expect(g.type).toBe("MultiPolygon");
    expect(g.coordinates).toHaveLength(2);
  });

  it("validates even when the fill is off, so the outline matches the shaded version", () => {
    const data = buildBoundaryLayer(bridged).props.data as GeoJSON.FeatureCollection;
    expect((data.features[0].geometry as GeoJSON.MultiPolygon).coordinates).toHaveLength(2);
  });

  it("fills only with emphasis", () => {
    expect(buildBoundaryLayer(bridged).props.filled).toBe(false);
    expect(buildBoundaryLayer(bridged, undefined, true).props.filled).toBe(true);
  });
});

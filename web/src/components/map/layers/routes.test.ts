import {describe, expect, it} from "vitest";

import {buildNamedRouteLayers, type NamedRoute} from "./routes";

const route = (over: Partial<NamedRoute>): NamedRoute => ({
  id: "r1",
  name: "R1",
  color: "#00ff00",
  points: [
    [40, -80],
    [41, -79],
  ],
  waypoints: [],
  ...over,
});

function byId(layers: ReturnType<typeof buildNamedRouteLayers>, id: string) {
  return layers.find((l) => l.id === id);
}

describe("buildNamedRouteLayers", () => {
  it("returns no layers for an empty route list", () => {
    expect(buildNamedRouteLayers([], null, new Set())).toEqual([]);
  });

  it("drops a degenerate (single-point) route but keeps a valid sibling", () => {
    const layers = buildNamedRouteLayers(
      [route({ id: "short", points: [[40, -80]] }), route({ id: "ok" })],
      null,
      new Set(),
    );
    const lines = byId(layers, "named-routes")!;
    expect((lines.props.data as { id?: string }[]).length).toBe(1);
  });

  it("returns no layers when every route is degenerate", () => {
    expect(
      buildNamedRouteLayers([route({ points: [[40, -80]] })], null, new Set()),
    ).toEqual([]);
  });

  it("omits the fix-dot/fix-label layers when no route is toggled labeled", () => {
    const layers = buildNamedRouteLayers([route({})], null, new Set());
    expect(layers).toHaveLength(3); // lines, endpoints, name labels — no fix layers
    expect(byId(layers, "named-route-fix-dots")).toBeUndefined();
  });

  it("adds fix-dot/fix-label layers only for routes toggled labeled", () => {
    const layers = buildNamedRouteLayers(
      [route({ id: "r1", waypoints: [{ name: "FIXA", lat: 40, lon: -80 }] })],
      null,
      new Set(["r1"]),
    );
    const fixDots = byId(layers, "named-route-fix-dots")!;
    expect((fixDots.props.data as unknown[]).length).toBe(1);
  });

  it("draws the selected route wider and fully opaque", () => {
    const layers = buildNamedRouteLayers(
      [route({ id: "a" }), route({ id: "b" })],
      "a",
      new Set(),
    );
    const lines = byId(layers, "named-routes")!;
    const props = lines.props as unknown as {
      getWidth: (d: { rgb: number[]; selected: boolean }) => number;
      getColor: (d: { rgb: number[]; selected: boolean }) => number[];
    };
    const rgb = [0, 255, 0];
    expect(props.getWidth({ rgb, selected: true })).toBeGreaterThan(
      props.getWidth({ rgb, selected: false }),
    );
    expect(props.getColor({ rgb, selected: true })[3]).toBeGreaterThan(
      props.getColor({ rgb, selected: false })[3],
    );
  });
});

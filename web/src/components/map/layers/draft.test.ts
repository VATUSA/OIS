import {describe, expect, it} from "vitest";

import {buildDraftLayers, type DraftLine} from "./draft";

function byId(layers: ReturnType<typeof buildDraftLayers>, id: string) {
  return layers.find((l) => l.id === id);
}

describe("buildDraftLayers", () => {
  it("renders only the vertex layer for an empty draft (no line)", () => {
    const draft: DraftLine = { color: "#ff0000", points: [] };
    const layers = buildDraftLayers(draft);
    expect(byId(layers, "draft-line")).toBeUndefined();
    const vertices = byId(layers, "draft-vertices")!;
    expect((vertices.props.data as unknown[]).length).toBe(0);
  });

  it("renders only the vertex layer for a single-point draft (not enough for a line)", () => {
    const draft: DraftLine = { color: "#ff0000", points: [[40, -80]] };
    const layers = buildDraftLayers(draft);
    expect(byId(layers, "draft-line")).toBeUndefined();
    const vertices = byId(layers, "draft-vertices")!;
    expect((vertices.props.data as unknown[]).length).toBe(1);
  });

  it("adds the dashed line once there are at least two points", () => {
    const draft: DraftLine = {
      color: "#ff0000",
      points: [
        [40, -80],
        [41, -79],
      ],
    };
    const layers = buildDraftLayers(draft);
    const line = byId(layers, "draft-line")!;
    expect(line).toBeDefined();
    expect(line.props.data).toEqual([{ path: [[-80, 40], [-79, 41]] }]);
    const vertices = byId(layers, "draft-vertices")!;
    expect((vertices.props.data as unknown[]).length).toBe(2);
  });
});

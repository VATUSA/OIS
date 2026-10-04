import {describe, expect, it} from "vitest";

import {handleVertices} from "./layers";

/** A ring of `n` distinct vertices, each identifiable by its position. */
const ring = (n: number): [number, number][] =>
  Array.from({length: n}, (_, i) => [-77 + i / 1000, 38 + i / 1000] as [number, number]);

describe("handleVertices (VATUSA/OIS#538)", () => {
  /**
   * The 98.8% case, and the one a cap must not touch. The dataset's median ring is 15 vertices
   * (`backend/data/faa_surface.json`, 24,583 rings), so almost every shape has to come back exactly
   * as it went in — a cap that quietly resampled ordinary shapes would be a regression dressed up
   * as a fix.
   */
  it("returns a short ring unchanged, with its own indices", () => {
    const path = ring(15);

    expect(handleVertices(path)).toEqual(path.map((pos, index) => ({pos, index})));
  });

  it("leaves a ring exactly at the cap alone", () => {
    expect(handleVertices(ring(80))).toHaveLength(80);
  });

  /** KDCA's main ramp, the shape in the issue's screenshot. */
  it("caps KDCA's 352-vertex ramp to far fewer handles", () => {
    const handles = handleVertices(ring(352));

    expect(handles.length).toBeLessThanOrEqual(80);
    expect(handles.length).toBeGreaterThan(40); // still a usable number, not a token few
  });

  /** AC2: the worst shape in the dataset must be usable, not merely better. */
  it("caps the dataset's worst case (KSFB, 1,920 vertices)", () => {
    expect(handleVertices(ring(1920)).length).toBeLessThanOrEqual(80);
  });

  /**
   * The assertion that matters most, and the one that would have caught the bug this fix could
   * easily have introduced.
   *
   * `SurfaceMap`'s drag assigns into `draft.points` by index. If a handle's `index` did not address
   * the vertex sitting at its own `pos`, dragging a handle would move a *different* vertex of the
   * ring — deforming the shape somewhere the user is not looking, with nothing on screen to show it
   * happened.
   */
  it("every handle's index addresses the vertex at its own position", () => {
    const path = ring(352);

    for (const {pos, index} of handleVertices(path)) {
      expect(path[index]).toEqual(pos);
    }
  });

  /**
   * A closed ring has to still read as closed, and the shape's extent must not appear to shrink —
   * so the two ends are never the vertices that get dropped.
   */
  it("always keeps the first and last vertex", () => {
    const path = ring(352);
    const handles = handleVertices(path);

    expect(handles[0]).toEqual({pos: path[0], index: 0});
    expect(handles[handles.length - 1]).toEqual({pos: path[351], index: 351});
  });

  it("samples in order and never repeats a vertex", () => {
    const indices = handleVertices(ring(1920)).map((h) => h.index);

    expect(indices).toEqual([...indices].sort((a, b) => a - b));
    expect(new Set(indices).size).toBe(indices.length);
  });

  /** Degenerate inputs reach this while a shape is still being drawn. */
  it("handles an empty or single-vertex path", () => {
    expect(handleVertices([])).toEqual([]);
    expect(handleVertices(ring(1))).toEqual([{pos: [-77, 38], index: 0}]);
  });
});

import {readFileSync} from "node:fs";
import {createRequire} from "node:module";
import path from "node:path";

import {describe, expect, it} from "vitest";

/**
 * VATUSA/OIS#775: map fills wedged outside their (correct) outlines after a data refresh.
 *
 * The geometry was never wrong. luma.gl 9.4.0 and 9.4.1 `Model.draw` sized an indexed draw from the
 * index buffer's byte length and ignored `setVertexCount` (luma.gl#3257). deck.gl's
 * `SolidPolygonLayer` only ever calls `setVertexCount`, and keeps its index buffer at its high-water
 * mark, so once a layer's data shrank (a TRACON or centre logging off, a smaller airport, a ramp
 * opened for editing) the stale tail of old indices was drawn too: fill triangles joining vertices
 * that belong to different polygons, while the separately-drawn outline stayed right. Reproduced in
 * headless Chromium against the real ATC and surface layer builders (7 of 22 refresh steps wedged
 * on 9.4.1, 0 on 9.4.2); an identical refetch with new data identity never wedged.
 *
 * The fix is luma.gl 9.4.2. These packages are transitive (no `package.json` names them), so only
 * the lockfile holds the version — this test fails if a lockfile change ever resolves deck.gl back
 * onto an engine with the bug. It reads the engine each deck.gl package actually resolves, not any
 * copy that happens to be installed.
 */

/** The luma.gl release that fixed it (luma.gl#3293). 9.3 did not have the bug, but deck.gl 9.4 needs
 * luma.gl ~9.4, so anything older than this is a broken 9.4 engine. */
const FIXED = [9, 4, 2] as const;

const fromWeb = createRequire(import.meta.url);

/** The `@luma.gl/engine` version that `deckPackage` resolves from its own location. */
function engineVersionFor(deckPackage: string): string {
  const fromDeck = createRequire(fromWeb.resolve(deckPackage));
  let dir = path.dirname(fromDeck.resolve("@luma.gl/engine"));
  for (;;) {
    try {
      const pkg = JSON.parse(readFileSync(path.join(dir, "package.json"), "utf8")) as {name?: string; version: string};
      if (pkg.name === "@luma.gl/engine") return pkg.version;
    } catch {
      // No package.json at this level; keep walking up.
    }
    const parent = path.dirname(dir);
    if (parent === dir) throw new Error(`no @luma.gl/engine package.json above ${deckPackage}'s resolution`);
    dir = parent;
  }
}

/** Whether `version` (`major.minor.patch[-pre]`) is at or past `FIXED`. */
function honoursVertexCount(version: string): boolean {
  const parts = version.split("-")[0].split(".").map(Number);
  for (let i = 0; i < FIXED.length; i++) {
    if (parts[i] !== FIXED[i]) return parts[i] > FIXED[i];
  }
  return true;
}

describe("luma.gl indexed draws honour vertexCount (#775)", () => {
  it("compares versions on every component, not just the patch", () => {
    expect(honoursVertexCount("9.4.1")).toBe(false);
    expect(honoursVertexCount("9.4.0")).toBe(false);
    expect(honoursVertexCount("9.3.9")).toBe(false);
    expect(honoursVertexCount("9.4.2")).toBe(true);
    expect(honoursVertexCount("9.4.10")).toBe(true);
    expect(honoursVertexCount("9.5.0")).toBe(true);
    expect(honoursVertexCount("10.0.0")).toBe(true);
  });

  // `@deck.gl/layers` owns SolidPolygonLayer; `@deck.gl/core` owns the device and the draw loop.
  // Each resolves its own copy under pnpm, so both are checked.
  it.each(["@deck.gl/core", "@deck.gl/layers"])("%s draws polygon fills on a fixed engine", (deckPackage) => {
    const version = engineVersionFor(deckPackage);
    expect(honoursVertexCount(version), `${deckPackage} resolves @luma.gl/engine ${version}`).toBe(true);
  });
});

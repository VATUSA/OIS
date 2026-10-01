import {describe, expect, it} from "vitest";

import {TEMPLATES, templatesFor} from "./templates";

/**
 * Asserts the real `templatesFor`, which `pages/dashboards/library` calls — not a copy of its rule.
 * A duplicated predicate here would stay green if the page stopped filtering.
 */
const ids = (tmuNational: boolean | undefined) => templatesFor(tmuNational).map((t) => t.id);

describe("who the NAS template is offered to (VATUSA/OIS#476)", () => {
  it("a national reader sees it", () => {
    expect(ids(true)).toContain("nas-watch");
  });

  it("a facility-scoped controller does not", () => {
    expect(ids(false)).not.toContain("nas-watch");
  });

  /**
   * `useMe` resolves after first render, so the flag is `undefined` before it does. Treating that as
   * "national" would flash the template to everyone; the library's memo also has to re-run when the
   * flag lands, which is why `templates` is a dependency rather than something read through the
   * handler ref.
   */
  it("is hidden while the profile is still loading", () => {
    expect(ids(undefined)).not.toContain("nas-watch");
  });

  it("hides nothing else from anyone", () => {
    const others = TEMPLATES.filter((t) => t.airports !== "national").map((t) => t.id);
    expect(ids(false)).toEqual(others);
    expect(ids(undefined)).toEqual(others);
    expect(ids(true)).toEqual([...others, "nas-watch"]);
  });
});

/**
 * The tests above prove `templatesFor` decides correctly. They cannot prove the page still *asks* it
 * — swapping the call for `templatesFor(true)` leaves every one of them green, which is the whole
 * failure mode a unit test of a predicate has.
 *
 * So this reads the call site, in the same spirit as `desktop-events.guard.test.ts` (#439): crude on
 * purpose, and aimed at the one thing that must not silently change.
 */
describe("the library actually applies the gate", () => {
  it("passes the signed-in user's flag to templatesFor, not a constant", async () => {
    const { readFile } = await import("node:fs/promises");
    const { resolve } = await import("node:path");
    const src = await readFile(
      resolve(process.cwd(), "src/pages/dashboards/library.tsx"),
      "utf8",
    );

    const call = /templatesFor\(([^)]*)\)/.exec(src);
    expect(call, "the library no longer calls templatesFor at all").not.toBeNull();
    expect(
      call?.[1],
      "the gate must read tmu_national from the profile, not a hardcoded value",
    ).toMatch(/tmu_national/);
  });
});

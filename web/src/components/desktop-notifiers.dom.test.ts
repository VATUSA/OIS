// @vitest-environment jsdom
import {describe, expect, it} from "vitest";

import {flattenPermissions} from "./desktop-notifiers";

/**
 * The flattener decides what counts as a *new* grant, so getting it wrong either misses grants or
 * announces revokes as grants. It walks the same tree shape `hasPermission` does.
 */
describe("flattenPermissions", () => {
  it("turns the nested tree into dotted permission names", () => {
    expect(
      flattenPermissions({flow: {fca: ["read", "update"]}, tmu: {program: ["read"]}}).sort(),
    ).toEqual(["flow.fca.read", "flow.fca.update", "tmu.program.read"]);
  });

  it("handles a tree deeper than two levels", () => {
    expect(flattenPermissions({a: {b: {c: ["do"]}}})).toEqual(["a.b.c.do"]);
  });

  it("is empty for a user with nothing", () => {
    expect(flattenPermissions({})).toEqual([]);
    expect(flattenPermissions(undefined)).toEqual([]);
    expect(flattenPermissions(null)).toEqual([]);
  });

  it("ignores leaves that aren't action arrays", () => {
    // A malformed tree must not produce junk names that look like grants.
    expect(flattenPermissions({flow: {fca: "nonsense"}})).toEqual([]);
  });

  it("produces a stable set, so only real additions look new", () => {
    const before = flattenPermissions({flow: {fca: ["read"]}});
    const after = flattenPermissions({flow: {fca: ["read", "update"]}});
    const added = after.filter((p) => !before.includes(p));
    expect(added).toEqual(["flow.fca.update"]);

    // And a revoke yields no additions — it must never read as a grant.
    const revoked = flattenPermissions({flow: {fca: []}});
    expect(revoked.filter((p) => !before.includes(p))).toEqual([]);
  });
});

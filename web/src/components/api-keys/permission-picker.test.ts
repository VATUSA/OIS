// VATUSA/OIS#550 AC4 + AC5: an API key is started from one of its creator's groups, not a preset.
//
// This file used to hold ~30 preset tests, nearly all for #264 and #275 — every one of them guarding
// `togglePresetSelection`'s attempt to reverse-engineer "is this preset applied?" from a selection.
// Those tests went with the function. What replaces them is a one-way merge with no applied state, so
// the bug class is gone rather than patched; the tests below pin the properties that make it so.
import {describe, expect, it} from "vitest";

import type {GrantablePermission} from "@/lib/api-keys";
import type {HeldGroup} from "@/lib/groups";

import {type PermSelection, mergeGroupIntoSelection} from "./permission-picker";

const national = (permission: string): GrantablePermission => ({ permission, national: true, artccs: [] });

const TRAFFIC = ["tmu.programs.update", "flow.fca.update", "stats.history.read"];
const ADMIN_GRANTABLE = [
  ...TRAFFIC,
  "ace.requests.create",
  "ace.requests.manage",
  "events.config.update",
  "access.users.update",
].map(national);
/** Holds the traffic domains and one ace permission — nothing else to delegate. */
const TMU_ONLY_GRANTABLE = [...TRAFFIC, "ace.requests.create"].map(national);
/** Holds everything, but only at ZDC. */
const ARTCC_LIMITED_GRANTABLE: GrantablePermission[] = ADMIN_GRANTABLE.map((g) => ({
  ...g,
  national: false,
  artccs: ["ZDC"],
}));

/** Whether a selected scope lies within what the creator can delegate. */
const withinBounds = (s: { national: boolean; artccs: string[] }, g: GrantablePermission) =>
  g.national || (!s.national && s.artccs.every((a) => g.artccs.includes(a)));

const group = (name: string, permissions: string[]): HeldGroup => ({ name, permissions });
const NTMO = group("NTMO", TRAFFIC);
const EVENTS = group("EVENTS_TEAM", ["events.config.update", "ace.requests.create", "ace.requests.manage"]);
const ADMIN = group("VATUSA_STAFF", ADMIN_GRANTABLE.map((g) => g.permission));

describe("mergeGroupIntoSelection (VATUSA/OIS#550)", () => {
  it("adds every permission the group grants that the creator can delegate", () => {
    const next = mergeGroupIntoSelection(NTMO, ADMIN_GRANTABLE, new Map());

    expect([...next.keys()].sort()).toEqual([...TRAFFIC].sort());
    for (const s of next.values()) expect(s).toEqual({ national: true, artccs: [] });
  });

  /** `grantable` is the cap. The server re-checks at creation, but the picker must not offer more. */
  it("skips what the creator cannot delegate", () => {
    const next = mergeGroupIntoSelection(EVENTS, TMU_ONLY_GRANTABLE, new Map());

    expect([...next.keys()]).toEqual(["ace.requests.create"]);
  });

  /**
   * **#275.** Stacking presets silently narrowed scopes, because applying one rewrote an existing
   * grant at its own scope. A merge must leave an existing entry exactly as broad as it was.
   *
   * The case that can actually narrow is the edit flow after the owner's access shrank: the key was
   * created when they held this permission nationally, they now hold it only at ZDC, so the default a
   * merge would write is ZDC. Overwriting would quietly cut the key down — the server is what caps a
   * key to its owner's live access, and it should, rather than the picker doing it unannounced.
   *
   * (My first version of this test limited every permission *except* this one, so the default it
   * would write equalled the existing grant and overwriting changed nothing. A mutation that made the
   * merge overwrite left it green — only `never widens` caught it.)
   */
  it("never narrows an existing grant (#275)", () => {
    const keyFromBefore: PermSelection = new Map([["tmu.programs.update", { national: true, artccs: [] }]]);
    const shrunk = ADMIN_GRANTABLE.map((g) =>
      g.permission === "tmu.programs.update" ? { ...g, national: false, artccs: ["ZDC"] } : g,
    );

    const next = mergeGroupIntoSelection(NTMO, shrunk, keyFromBefore);

    expect(next.get("tmu.programs.update")).toEqual({ national: true, artccs: [] });
  });

  /** ...nor widens one. An existing entry is the user's choice; a template only fills gaps. */
  it("never widens an existing grant either", () => {
    const narrow: PermSelection = new Map([["tmu.programs.update", { national: false, artccs: ["ZDC"] }]]);

    const next = mergeGroupIntoSelection(NTMO, ADMIN_GRANTABLE, narrow);

    expect(next.get("tmu.programs.update")).toEqual({ national: false, artccs: ["ZDC"] });
  });

  it("never removes anything already selected", () => {
    const sel: PermSelection = new Map([["access.users.update", { national: true, artccs: [] }]]);

    const next = mergeGroupIntoSelection(NTMO, ADMIN_GRANTABLE, sel);

    expect(next.get("access.users.update")).toEqual({ national: true, artccs: [] });
    expect(next.size).toBe(TRAFFIC.length + 1);
  });

  /**
   * **#264.** "Is this preset applied?" was inferred from the selection, and the inference was wrong.
   * A merge has no applied state to infer: doing it twice is the same as doing it once, so there is
   * nothing for a UI to light up or to get wrong.
   */
  it("is idempotent — there is no applied state to infer (#264)", () => {
    const once = mergeGroupIntoSelection(ADMIN, ADMIN_GRANTABLE, new Map());
    const twice = mergeGroupIntoSelection(ADMIN, ADMIN_GRANTABLE, once);

    expect([...twice]).toEqual([...once]);
  });

  it("does not mutate the selection it was given", () => {
    const sel: PermSelection = new Map();

    mergeGroupIntoSelection(NTMO, ADMIN_GRANTABLE, sel);

    expect(sel.size).toBe(0);
  });

  /**
   * Kept from the preset suite, because it still matters: whatever order a creator starts from
   * groups in, nothing selected may exceed what they can delegate. An ARTCC-limited creator is the
   * case that would catch a merge defaulting to national.
   */
  it.each([
    ["admin", ADMIN_GRANTABLE],
    ["TMU-only", TMU_ONLY_GRANTABLE],
    ["ARTCC-limited", ARTCC_LIMITED_GRANTABLE],
  ] as const)("for a %s creator, every sequence of merges stays within bounds", (_name, grantable) => {
    const byName = new Map(grantable.map((g) => [g.permission, g] as const));
    const groups = [NTMO, EVENTS, ADMIN];
    const orders = groups.flatMap((a) => groups.flatMap((b) => groups.map((c) => [a, b, c])));

    for (const order of orders) {
      let sel: PermSelection = new Map();
      for (const g of order) sel = mergeGroupIntoSelection(g, grantable, sel);
      for (const [perm, s] of sel) {
        const grant = byName.get(perm);
        expect(grant && withinBounds(s, grant), `${perm} out of bounds after ${order.map((g) => g.name).join(" → ")}`).toBe(
          true,
        );
      }
    }
  });
});

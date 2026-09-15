import {describe, expect, it} from "vitest";

import type {GrantablePermission} from "@/lib/api-keys";
import {ACCESS_PRESETS, BASE_PERMISSIONS} from "@/lib/presets";

import {type PermSelection, presetApplied, presetOwnPermissions} from "./permission-picker";

const preset = (id: string) => ACCESS_PRESETS.find((p) => p.id === id)!;
const national = (permission: string): GrantablePermission => ({ permission, national: true, artccs: [] });

const TRAFFIC = ["tmu.programs.update", "flow.fca.update", "stats.history.read"];
const ADMIN_GRANTABLE = [
  ...TRAFFIC,
  "ace.requests.create",
  "ace.requests.manage",
  "events.config.update",
  "access.users.update",
].map(national);
// A TMU-only creator: the traffic domains plus the sign-in baseline — nothing else to delegate.
const TMU_ONLY_GRANTABLE = [...TRAFFIC, "ace.requests.create"].map(national);
const base = (grantable: GrantablePermission[]) =>
  BASE_PERMISSIONS.filter((p) => grantable.some((g) => g.permission === p));

const selectNationally = (names: string[]): PermSelection =>
  new Map(names.map((n) => [n, { national: true, artccs: [] }]));

/** The national presets that read as applied for a creator after selecting `names`. */
function applied(grantable: GrantablePermission[], names: string[], facility = "") {
  const sel = selectNationally(names);
  return ACCESS_PRESETS.filter((p) => presetApplied(p, grantable, base(grantable), facility, sel)).map(
    (p) => p.id,
  );
}

describe("preset highlighting (#264)", () => {
  it("an admin clicking NTMO lights only NTMO", () => {
    expect(applied(ADMIN_GRANTABLE, [...TRAFFIC, "ace.requests.create"])).toEqual(["ntmo"]);
  });

  it("a TMU-only creator's NTMO click doesn't light presets that grant nothing of their own", () => {
    const lit = applied(TMU_ONLY_GRANTABLE, [...TRAFFIC, "ace.requests.create"]);
    // Identical grant sets for this creator — genuinely all applied.
    expect(lit).toEqual(expect.arrayContaining(["vatusa_admin", "dcc_staff", "ntmo"]));
    // Only the shared baseline overlaps these — the pre-#264 bug lit them.
    expect(lit).not.toContain("ace_team");
    expect(lit).not.toContain("events_team");
    expect(lit).not.toContain("facility_ec");
    expect(lit).not.toContain("facility_aec");
  });

  it("a preset with nothing of its own to grant has an empty own set", () => {
    const g = TMU_ONLY_GRANTABLE;
    expect(presetOwnPermissions(preset("ace_team"), g, base(g), "")).toEqual([]);
    expect(presetOwnPermissions(preset("events_team"), g, base(g), "")).toEqual([]);
    expect(presetOwnPermissions(preset("ntmo"), g, base(g), "")).toEqual(TRAFFIC);
  });

  it("the baseline alone never makes a preset applied", () => {
    expect(applied(ADMIN_GRANTABLE, ["ace.requests.create"])).toEqual([]);
  });

  it("a facility preset is applied only when its perms are scoped to the chosen facility", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const own = presetOwnPermissions(ec, g, base(g), "ZDC");
    expect(own.length).toBeGreaterThan(0);
    expect(presetOwnPermissions(ec, g, base(g), "")).toEqual([]);

    const nationalSel = selectNationally([...own, "ace.requests.create"]);
    expect(presetApplied(ec, g, base(g), "ZDC", nationalSel)).toBe(false);
    expect(presetApplied(ec, g, base(g), "", nationalSel)).toBe(false);

    const scoped: PermSelection = new Map(own.map((n) => [n, { national: false, artccs: ["ZDC"] }]));
    scoped.set("ace.requests.create", { national: true, artccs: [] });
    expect(presetApplied(ec, g, base(g), "ZDC", scoped)).toBe(true);
    expect(presetApplied(ec, g, base(g), "ZNY", scoped)).toBe(false);
  });

  it("a facility preset only counts perms the creator can delegate at that facility", () => {
    const g: GrantablePermission[] = [
      { permission: "tmu.programs.update", national: false, artccs: ["ZDC"] },
      { permission: "flow.fca.update", national: false, artccs: ["ZNY"] },
    ];
    expect(presetOwnPermissions(preset("facility_ec"), g, [], "ZDC")).toEqual(["tmu.programs.update"]);
  });
});

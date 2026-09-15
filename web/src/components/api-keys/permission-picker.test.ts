import {describe, expect, it} from "vitest";

import type {GrantablePermission} from "@/lib/api-keys";
import {ACCESS_PRESETS, BASE_PERMISSIONS} from "@/lib/presets";

import {
  type PermSelection,
  presetApplied,
  presetCanApply,
  presetOwnPermissions,
  togglePresetSelection,
} from "./permission-picker";

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

  // Admin applies Facility EC at ZDC: every operational perm ARTCC-scoped, baseline national.
  const ecAtZdc = (g: GrantablePermission[]): PermSelection =>
    togglePresetSelection(preset("facility_ec"), g, base(g), "ZDC", new Map());

  it("a facility preset's ARTCC-scoped grants don't light any national preset", () => {
    const g = ADMIN_GRANTABLE;
    const sel = ecAtZdc(g);
    const lit = ACCESS_PRESETS.filter((p) => presetApplied(p, g, base(g), "ZDC", sel)).map((p) => p.id);
    // AEC grants the identical set at ZDC, so it genuinely reads as applied too.
    expect(lit).toEqual(["facility_ec", "facility_aec"]);
  });

  it("clicking a national preset after a facility preset grants it nationally, not strips it", () => {
    const g = ADMIN_GRANTABLE;
    const next = togglePresetSelection(preset("ntmo"), g, base(g), "ZDC", ecAtZdc(g));
    for (const p of TRAFFIC) expect(next.get(p)).toEqual({ national: true, artccs: [] });
    expect(next.get("ace.requests.create")).toEqual({ national: true, artccs: [] });
    expect(presetApplied(preset("ntmo"), g, base(g), "ZDC", next)).toBe(true);
  });

  it("clicking an applied preset removes its perms and the baseline", () => {
    const g = ADMIN_GRANTABLE;
    const on = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    const off = togglePresetSelection(preset("ntmo"), g, base(g), "", on);
    expect([...off.keys()]).toEqual([]);
  });

  it("a facility preset after a national preset keeps the national grants (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ntmo = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    const next = togglePresetSelection(preset("facility_ec"), g, base(g), "ZDC", ntmo);
    for (const p of TRAFFIC) expect(next.get(p)).toEqual({ national: true, artccs: [] });
  });

  it("stacking a facility preset at two ARTCCs merges, and removing one leaves the other (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const zny = togglePresetSelection(ec, g, base(g), "ZNY", new Map());
    const both = togglePresetSelection(ec, g, base(g), "ZDC", zny);
    for (const p of TRAFFIC) expect(both.get(p)).toEqual({ national: false, artccs: ["ZNY", "ZDC"] });

    const off = togglePresetSelection(ec, g, base(g), "ZDC", both);
    for (const p of TRAFFIC) expect(off.get(p)).toEqual({ national: false, artccs: ["ZNY"] });
    expect(off.has("ace.requests.create")).toBe(true); // EC still applied at ZNY needs it
    expect(presetApplied(ec, g, base(g), "ZNY", off)).toBe(true);
  });

  it("removing a preset keeps the baseline another applied preset still needs (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ace = togglePresetSelection(preset("ace_team"), g, base(g), "", new Map());
    const withNtmo = togglePresetSelection(preset("ntmo"), g, base(g), "", ace);
    const off = togglePresetSelection(preset("ntmo"), g, base(g), "", withNtmo);
    for (const p of TRAFFIC) expect(off.has(p)).toBe(false);
    expect(off.has("ace.requests.create")).toBe(true);
    expect(presetApplied(preset("ace_team"), g, base(g), "", off)).toBe(true);
  });

  it("re-merging a facility already listed doesn't duplicate it (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const sel: PermSelection = new Map([["tmu.programs.update", { national: false, artccs: ["ZDC"] }]]);
    const next = togglePresetSelection(preset("facility_ec"), g, base(g), "ZDC", sel);
    expect(next.get("tmu.programs.update")).toEqual({ national: false, artccs: ["ZDC"] });
  });

  it("removing a preset inside a lit one still turns it off; the containing one goes unlit (#275)", () => {
    const g = ADMIN_GRANTABLE;
    // DCC Staff alone already lights NTMO (its grants lie inside DCC Staff's).
    const both = togglePresetSelection(preset("dcc_staff"), g, base(g), "", new Map());
    expect(presetApplied(preset("ntmo"), g, base(g), "", both)).toBe(true);
    const off = togglePresetSelection(preset("ntmo"), g, base(g), "", both);
    // Accepted cost (operator decision): DCC Staff + NTMO is indistinguishable from DCC Staff alone,
    // so turning NTMO off removes its perms and DCC Staff goes unlit rather than ignoring the click.
    expect(presetApplied(preset("ntmo"), g, base(g), "", off)).toBe(false);
    expect(presetApplied(preset("dcc_staff"), g, base(g), "", off)).toBe(false);
    for (const p of TRAFFIC) expect(off.has(p)).toBe(false);
  });

  it("a preset lit alongside a containing one it lit up turns off when clicked (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ntmo = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    const both = togglePresetSelection(preset("events_team"), g, base(g), "", ntmo);
    // NTMO + Events Team is DCC Staff's whole set here, so DCC Staff (and ACE Team) light too.
    expect(presetApplied(preset("dcc_staff"), g, base(g), "", both)).toBe(true);
    for (const id of ["ntmo", "events_team"]) {
      const off = togglePresetSelection(preset(id), g, base(g), "", both);
      expect(presetApplied(preset(id), g, base(g), "", off)).toBe(false);
    }
  });

  it("removing a facility preset keeps the baseline while national grants remain (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const ntmo = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    ntmo.set("flow.fca.update", { national: false, artccs: ["ZDC"] }); // narrowed by hand
    const on = togglePresetSelection(ec, g, base(g), "ZDC", ntmo);
    expect(presetApplied(ec, g, base(g), "ZDC", on)).toBe(true);
    const off = togglePresetSelection(ec, g, base(g), "ZDC", on);
    expect(off.get("tmu.programs.update")).toEqual({ national: true, artccs: [] });
    expect(off.get("stats.history.read")).toEqual({ national: true, artccs: [] });
    expect(off.has("ace.requests.create")).toBe(true);
  });

  it("removing a national preset stacked on a facility preset hands the facility its ARTCC back (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const ntmo = preset("ntmo");
    const ecFirst = togglePresetSelection(ntmo, g, base(g), "ZDC", ecAtZdc(g));
    const ntmoFirst = togglePresetSelection(ec, g, base(g), "ZDC", togglePresetSelection(ntmo, g, base(g), "", new Map()));
    for (const on of [ecFirst, ntmoFirst]) {
      const off = togglePresetSelection(ntmo, g, base(g), "ZDC", on);
      for (const p of TRAFFIC) expect(off.get(p)).toEqual({ national: false, artccs: ["ZDC"] });
      expect(off.get("ace.requests.create")).toEqual({ national: true, artccs: [] });
      expect(presetApplied(ec, g, base(g), "ZDC", off)).toBe(true);
    }
  });

  it("removing a national preset keeps a facility preset applied at an ARTCC other than the dropdown's (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const zny = togglePresetSelection(preset("facility_ec"), g, base(g), "ZNY", new Map());
    const on = togglePresetSelection(preset("ace_team"), g, base(g), "ZDC", zny);
    const off = togglePresetSelection(preset("ace_team"), g, base(g), "ZDC", on);
    expect(off.get("ace.requests.manage")).toEqual({ national: false, artccs: ["ZNY"] });
    expect(presetApplied(preset("facility_ec"), g, base(g), "ZNY", off)).toBe(true);
  });

  it("a facility preset stacked on national grants reads as applied and removes only its ARTCC grants (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const ntmo = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    expect(presetApplied(ec, g, base(g), "ZDC", ntmo)).toBe(false); // nothing of its own at ZDC yet
    const on = togglePresetSelection(ec, g, base(g), "ZDC", ntmo);
    expect(presetApplied(ec, g, base(g), "ZDC", on)).toBe(true);
    const off = togglePresetSelection(ec, g, base(g), "ZDC", on);
    expect(off).toEqual(ntmo);
  });

  it("removing a facility preset leaves a hand-picked national grant national (#275)", () => {
    const g = ADMIN_GRANTABLE;
    const ec = preset("facility_ec");
    const sel: PermSelection = new Map([["tmu.programs.update", { national: true, artccs: [] }]]);
    const on = togglePresetSelection(ec, g, base(g), "ZDC", sel);
    expect(presetApplied(ec, g, base(g), "ZDC", on)).toBe(true);
    const off = togglePresetSelection(ec, g, base(g), "ZDC", on);
    // The national grant is left alone; the baseline stays with it, since a remaining own perm keeps
    // the baseline (the selection can't tell a hand-picked grant from one a preset left, #275).
    expect(off).toEqual(new Map([...sel, ["ace.requests.create", { national: true, artccs: [] }]]));
  });

  it("removing a preset also removes presets wholly inside it, which can't be told apart (#275)", () => {
    const admin = ADMIN_GRANTABLE;
    const all = togglePresetSelection(preset("vatusa_admin"), admin, base(admin), "", new Map());
    expect([...togglePresetSelection(preset("vatusa_admin"), admin, base(admin), "", all).keys()]).toEqual([]);
    // EC and AEC grant identical sets at ZDC.
    expect([...togglePresetSelection(preset("facility_ec"), admin, base(admin), "ZDC", ecAtZdc(admin)).keys()]).toEqual([]);
    // For a TMU-only creator, NTMO, DCC Staff and VATUSA Admin grant identical sets.
    const tmu = TMU_ONLY_GRANTABLE;
    const on = togglePresetSelection(preset("ntmo"), tmu, base(tmu), "", new Map());
    expect([...togglePresetSelection(preset("ntmo"), tmu, base(tmu), "", on).keys()]).toEqual([]);
  });

  it("a national preset narrowed to one ARTCC no longer reads as applied", () => {
    const g = ADMIN_GRANTABLE;
    const sel = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    sel.set("flow.fca.update", { national: false, artccs: ["ZDC"] });
    expect(presetApplied(preset("ntmo"), g, base(g), "", sel)).toBe(false);
  });

  it("a national preset for a creator holding only ARTCCs is applied at all of those ARTCCs", () => {
    const g: GrantablePermission[] = TRAFFIC.map((permission) => ({ permission, national: false, artccs: ["ZDC", "ZNY"] }));
    const all = togglePresetSelection(preset("ntmo"), g, [], "", new Map());
    expect(all.get("tmu.programs.update")).toEqual({ national: false, artccs: ["ZDC", "ZNY"] });
    expect(presetApplied(preset("ntmo"), g, [], "", all)).toBe(true);
    all.set("tmu.programs.update", { national: false, artccs: ["ZDC"] });
    expect(presetApplied(preset("ntmo"), g, [], "", all)).toBe(false);
  });

  it("a preset isn't applied while the baseline is missing", () => {
    const g = ADMIN_GRANTABLE;
    const sel = togglePresetSelection(preset("ntmo"), g, base(g), "", new Map());
    sel.delete("ace.requests.create");
    expect(presetApplied(preset("ntmo"), g, base(g), "", sel)).toBe(false);
  });

  it("only presets with something of their own to grant are enabled", () => {
    const g = TMU_ONLY_GRANTABLE;
    expect(presetCanApply(preset("ntmo"), g, base(g), "")).toBe(true);
    expect(presetCanApply(preset("ace_team"), g, base(g), "")).toBe(false);
    // No facility yet: left enabled here, PresetBar shows "Pick a facility first".
    expect(presetCanApply(preset("facility_ec"), g, base(g), "")).toBe(true);
    expect(presetCanApply(preset("facility_ec"), [], [], "ZDC")).toBe(false);
  });
});

// ARTCC-limited creator: every grantable perm, the baseline included, only at ZDC.
const ARTCC_LIMITED_GRANTABLE: GrantablePermission[] = ADMIN_GRANTABLE.map((g) => ({
  ...g,
  national: false,
  artccs: ["ZDC"],
}));

/** Whether a selected scope lies within what the creator can delegate. */
const withinBounds = (s: { national: boolean; artccs: string[] }, g: GrantablePermission) =>
  g.national || (!s.national && s.artccs.every((a) => g.artccs.includes(a)));

describe("preset click invariants (#275)", () => {
  const FACILITIES = ["ZDC", "ZNY"];
  const clicks = ACCESS_PRESETS.flatMap((p) =>
    p.scope === "facility" ? FACILITIES.map((f) => ({ p, f })) : [{ p, f: "" }],
  );
  const stateKey = (sel: PermSelection) =>
    JSON.stringify([...sel].sort(([a], [b]) => a.localeCompare(b)));
  const label = (path: string[]) => path.join(" → ") || "(empty)";

  it.each([
    ["admin", ADMIN_GRANTABLE],
    ["TMU-only", TMU_ONLY_GRANTABLE],
    ["ARTCC-limited", ARTCC_LIMITED_GRANTABLE],
  ] as const)(
    "for a %s creator, every click stays within bounds and every lit chip turns off",
    (_name, g) => {
      const byName = new Map(g.map((x) => [x.permission, x] as const));
      const b = base(g);
      let frontier: { sel: PermSelection; path: string[] }[] = [{ sel: new Map(), path: [] }];
      const seen = new Set([stateKey(new Map())]);

      for (let depth = 0; depth <= 4; depth++) {
        const nextFrontier: typeof frontier = [];
        for (const { sel, path } of frontier) {
          for (const [perm, s] of sel) {
            const grant = byName.get(perm);
            expect(grant && withinBounds(s, grant), `${perm} out of bounds after ${label(path)}`).toBe(true);
          }
          for (const { p, f } of clicks) {
            const after = togglePresetSelection(p, g, b, f, sel);
            const step = [...path, f ? `${p.id}@${f}` : p.id];
            if (presetApplied(p, g, b, f, sel)) {
              expect(presetApplied(p, g, b, f, after), `${label(step)} left ${p.id} lit`).toBe(false);
            }
            const key = stateKey(after);
            if (depth < 4 && !seen.has(key)) {
              seen.add(key);
              nextFrontier.push({ sel: after, path: step });
            }
          }
        }
        frontier = nextFrontier;
      }
    },
  );
});

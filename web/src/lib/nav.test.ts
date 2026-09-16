import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {
  AREAS,
  areaById,
  areaForPath,
  canOpenPath,
  canSeeAdmin,
  canSeeArea,
  canSeeItem,
  groupForPath,
  itemForPath,
  visibleGroups,
} from "./nav";

function me(permissions: Record<string, unknown>, server_admin = false): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin,
    role_names: [],
    permissions: permissions as Me["permissions"],
  } as Me;
}

/** A permission tree holding exactly `names` (dotted `segments.action`). */
function holding(...names: string[]): Me {
  const tree: Record<string, unknown> = {};
  for (const name of names) {
    const parts = name.split(".");
    const action = parts.pop()!;
    let node = tree;
    for (const [i, seg] of parts.entries()) {
      if (i === parts.length - 1) node[seg] = [...((node[seg] as string[]) ?? []), action];
      else node = (node[seg] ??= {}) as Record<string, unknown>;
    }
  }
  return me(tree);
}

/**
 * Every nav link and the permission(s) its page's API requires, written out independently of
 * `AREAS` (the backend `RequirePermission` markers are the source). `[]` = public.
 */
const REQUIRED: Record<string, readonly string[]> = {
  "/advisories": [],
  "/advisories/fcas": [],
  "/facility-map": [],
  "/pilot": [],
  "/ops/airport": ["tmu.program.read"],
  "/ops/tmu": ["tmu.program.read", "tmu.tmi.read", "flow.fca.read", "flow.runway.read"],
  "/ops/my": ["tmu.program.read"],
  "/ops/fca": ["flow.fca.read"],
  "/ops/idst": ["flow.fca.read"],
  "/ops/runway": ["flow.runway.read"],
  "/ops/aadc": ["tmu.program.read"],
  "/admin/planning/events": ["events.plan.read"],
  "/admin/planning/airport-configs": ["events.plan.read"],
  "/admin/planning/facility-documents": ["facilities.docs.read"],
  "/admin/planning/airport-surface": ["events.plan.read"],
  "/admin/planning/aircraft-profiles": ["flow.aircraft_profiles.read"],
  "/admin/historical": ["stats.data.read"],
  "/admin/historical/dashboard": ["stats.data.read"],
  "/admin/historical/replay": ["stats.data.read"],
  "/admin/historical/delays": ["stats.data.read"],
  "/admin/historical/taxi": ["stats.data.read"],
  "/admin/access": ["access.users.read"],
  "/admin/audit": ["audit.logs.read"],
  "/admin/jobs": ["system.jobs.read"],
  "/admin/api-keys": ["api_keys.key.read"],
  "/admin/discord": ["discord.config.read"],
};

const ALL_REQUIRED = [...new Set(Object.values(REQUIRED).flat())];
const ITEMS = AREAS.flatMap((a) => a.groups.flatMap((g) => g.items));

const labels = (u: Me | null, id: "advisories" | "operations" | "admin") =>
  visibleGroups(u, areaById(id)).flatMap((g) => g.items.map((i) => i.label));

describe("nav gating", () => {
  it("declares a required permission for every nav link, and no stale ones", () => {
    expect(ITEMS.map((i) => i.to).sort()).toEqual(Object.keys(REQUIRED).sort());
  });

  for (const [to, required] of Object.entries(REQUIRED)) {
    it(`${to} is shown exactly to holders of ${required.join(" | ") || "nothing (public)"}`, () => {
      const item = ITEMS.find((i) => i.to === to)!;
      if (required.length === 0) {
        expect(canSeeItem(null, item)).toBe(true);
        return;
      }
      expect(canSeeItem(null, item)).toBe(false);
      // Every other link's permission, but none of this one's.
      expect(canSeeItem(holding(...ALL_REQUIRED.filter((p) => !required.includes(p))), item)).toBe(false);
      for (const permission of required) expect(canSeeItem(holding(permission), item)).toBe(true);
    });
  }

  it("shows anonymous users only the public Advisories area", () => {
    expect(labels(null, "advisories")).toEqual(["Advisories", "FCAs", "Facility Map", "Pilot"]);
    expect(canSeeArea(null, areaById("operations"))).toBe(false);
    expect(canSeeAdmin(null)).toBe(false);
  });

  it("gives a planner the Planning links they can use, and the Admin page", () => {
    const planner = me({ events: { plan: ["read"] } });
    expect(labels(planner, "admin")).toEqual(["Events", "Airport Configs", "Airport Surface"]);
    expect(visibleGroups(planner, areaById("admin")).map((g) => g.label)).toEqual(["Planning"]);
    expect(canSeeAdmin(planner)).toBe(true);
  });

  it("gates document and aircraft-profile links on their own permissions", () => {
    const docs = me({ facilities: { docs: ["read"] } });
    expect(labels(docs, "admin")).toEqual(["Facility Documents"]);
  });

  it("gives a stats reader only Historical", () => {
    const stats = me({ stats: { data: ["read"] } });
    expect(visibleGroups(stats, areaById("admin")).map((g) => g.label)).toEqual(["Historical"]);
  });

  it("shows Operations' TMU to any ops reader, the rest per permission", () => {
    const runway = me({ flow: { runway: ["read"] } });
    expect(labels(runway, "operations")).toEqual(["TMU", "Runway"]);
  });

  it("includes the jobs link that the old ADMIN_PERMISSIONS missed", () => {
    expect(labels(me({ system: { jobs: ["read"] } }), "admin")).toEqual(["Jobs"]);
  });

  it("gives a server admin everything", () => {
    const admin = me({}, true);
    for (const area of AREAS) {
      const all = area.groups.flatMap((g) => g.items).length;
      expect(visibleGroups(admin, area).flatMap((g) => g.items)).toHaveLength(all);
    }
  });
});

describe("canOpenPath", () => {
  it("opens an admin page only to holders of that page's permission", () => {
    const planner = holding("events.plan.read");
    expect(canOpenPath(planner, "/admin/planning/events/123")).toBe(true);
    expect(canOpenPath(planner, "/admin")).toBe(true);
    for (const page of ["/admin/access", "/admin/audit", "/admin/jobs", "/admin/api-keys", "/admin/discord"]) {
      expect(canOpenPath(planner, page)).toBe(false);
    }
    expect(canOpenPath(holding("audit.logs.read"), "/admin/audit")).toBe(true);
  });

  it("keeps the Admin page closed to users with no Admin-area link", () => {
    const none = holding("tmu.program.read");
    expect(canOpenPath(none, "/admin")).toBe(false);
    expect(canOpenPath(null, "/admin/historical/flights/abc")).toBe(false);
  });

  it("defers pages below a group but not a nav item to their own checks", () => {
    expect(canOpenPath(holding("events.plan.read"), "/admin/historical/flights/abc")).toBe(true);
  });
});

describe("path lookup", () => {
  it("maps paths to areas and the most specific item", () => {
    expect(areaForPath("/facility-map/ZNY")?.id).toBe("advisories");
    expect(areaForPath("/ops")?.id).toBe("operations");
    expect(areaForPath("/opsx")).toBeUndefined();
    expect(itemForPath("/admin/planning/events/123")?.item.label).toBe("Events");
    expect(itemForPath("/admin/historical")?.item.label).toBe("Overview");
    expect(itemForPath("/admin/historical/flights/abc")).toBeUndefined();
    expect(itemForPath("/advisories/fcas")?.item.label).toBe("FCAs");
    expect(itemForPath("/advisories")?.item.label).toBe("Advisories");
    expect(groupForPath("/admin/historical/flights/abc")?.label).toBe("Historical");
    expect(groupForPath("/admin/access")).toBeUndefined();
  });
});

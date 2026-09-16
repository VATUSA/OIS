import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {AREAS, areaById, areaForPath, canSeeAdmin, canSeeArea, itemForPath, visibleGroups} from "./nav";

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

const labels = (u: Me | null, id: "advisories" | "operations" | "admin") =>
  visibleGroups(u, areaById(id)).flatMap((g) => g.items.map((i) => i.label));

describe("nav gating", () => {
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
  });
});

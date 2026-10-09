// @vitest-environment jsdom
import {describe, expect, it} from "vitest";

import {AREAS, itemForPath} from "./lib/nav";
import {SectorMonitorPage} from "./pages/sector-monitor";
import {router} from "./router";

/**
 * #725 AC "the page is under Operations and reachable from its nav", and epic #720 "it lives under
 * Operations, not Flow". The nav item and the route are declared in two files; this pins that they
 * agree, so moving either one (or dropping the route) fails here rather than leaving a dead link.
 */
describe("the Sector Monitor's place (#725)", () => {
  it("is a route under /ops that mounts the page", () => {
    const route = router.routesByPath["/ops/sectors" as keyof typeof router.routesByPath];
    expect(route, "registered").toBeDefined();
    expect(route.options.component).toBe(SectorMonitorPage);
  });

  it("is linked from the Operations area's nav, on the page's own read permission", () => {
    const hit = itemForPath("/ops/sectors");
    expect(hit?.area.id).toBe("operations");
    expect(hit?.item).toMatchObject({label: "Sector Monitor", to: "/ops/sectors", permission: "flow.sectors.read"});
  });

  it("is linked from no other area", () => {
    const linking = AREAS.filter((a) => a.groups.some((g) => g.items.some((i) => i.to === "/ops/sectors"))).map(
      (a) => a.id,
    );
    expect(linking).toEqual(["operations"]);
  });
});

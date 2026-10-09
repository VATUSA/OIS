// @vitest-environment jsdom
import {describe, expect, it} from "vitest";

import {router} from "./router";

/**
 * The Airspace Monitor was removed (#719): its two pages are no longer routes, so their old URLs fall
 * through to the router's normal not-found rather than a page that errors. The sector viewer, which
 * was kept as the dataset's inspector, still is a route — so this can't pass by reading nothing.
 */
describe("removed Monitor routes (#719)", () => {
  const paths = Object.keys(router.routesByPath);

  it("no longer registers the Monitor or its alert-parameter page", () => {
    expect(paths).not.toContain("/admin/flow/monitor");
    expect(paths).not.toContain("/admin/planning/sector-maps");
  });

  it("keeps the sector viewer", () => {
    expect(paths).toContain("/admin/flow/sectors");
  });
});

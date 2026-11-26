// @vitest-environment jsdom
import {isRedirect} from "@tanstack/react-router";
import {afterEach, describe, expect, it} from "vitest";

import type {Me} from "./lib/auth";
import {areaById, type NavItem} from "./lib/nav";
import {queryClient} from "./lib/query-client";
import {navGuard} from "./lib/route-guard";
import {router} from "./router";

/**
 * #1121 AC5: every Operations page refuses a member who could not see its nav link, so a page hidden
 * from the sidebar cannot be opened by typing its URL. Driven through the real route tree's
 * `beforeLoad`, reading `["me"]` from the app's own query cache as the router does.
 */

/** A member holding exactly `permission` (a dotted name), never a server admin. */
function holding(permission: string | null): Me {
  const permissions: Record<string, unknown> = {};
  if (permission) {
    const parts = permission.split(".");
    const action = parts.pop()!;
    let node = permissions;
    for (const segment of parts.slice(0, -1)) node = (node[segment] ??= {}) as Record<string, unknown>;
    node[parts[parts.length - 1]] = [action];
  }
  return {server_admin: false, permissions} as unknown as Me;
}

/** Every page route under `/ops` and the nav item whose permission guards it. */
const OPS_ITEMS: NavItem[] = areaById("operations").groups.flatMap((g) => g.items);
/** The routes a nav item's guard covers: the item's own path, and a dashboard board. */
const ROUTES_FOR: Record<string, string[]> = {"/ops/my": ["/ops/my", "/ops/my/$boardId"]};

async function runGuard(path: string): Promise<unknown> {
  const route = router.routesByPath[path as keyof typeof router.routesByPath];
  expect(route, `${path} is registered`).toBeDefined();
  const beforeLoad = (route.options as {beforeLoad?: (ctx: unknown) => unknown}).beforeLoad;
  expect(beforeLoad, `${path} has a beforeLoad`).toBeTypeOf("function");
  try {
    await beforeLoad!({});
    return "allowed";
  } catch (e) {
    return e;
  }
}

afterEach(() => queryClient.clear());

describe("Operations routes check the nav permission (#1121)", () => {
  it("covers every Operations nav item", () => {
    expect(OPS_ITEMS.map((i) => i.to)).toEqual([
      "/ops/airport",
      "/ops/tmu",
      "/ops/my",
      "/ops/fca",
      "/ops/idst",
      "/ops/runway",
      "/ops/aadc",
      "/ops/sectors",
    ]);
  });

  for (const item of OPS_ITEMS) {
    const permission = item.permission ?? item.anyOf![0];
    for (const path of ROUTES_FOR[item.to] ?? [item.to]) {
      it(`${path}: refuses a member without ${permission}, a signed-out visitor, and opens for a holder`, async () => {
        queryClient.setQueryData(["me"], holding(null));
        const refused = await runGuard(path);
        expect(isRedirect(refused), `${path} without the permission`).toBe(true);
        expect((refused as {options: {to: string}}).options.to).toBe("/");

        queryClient.setQueryData(["me"], null);
        expect(isRedirect(await runGuard(path)), `${path} signed out`).toBe(true);

        queryClient.setQueryData(["me"], holding(permission));
        expect(await runGuard(path)).toBe("allowed");
      });
    }
  }

  it("lets a member holding only the USER baseline's flow.fca.read open FCA and IDST, and nothing else", async () => {
    queryClient.setQueryData(["me"], holding("flow.fca.read"));
    const open: string[] = [];
    for (const item of OPS_ITEMS) if ((await runGuard(item.to)) === "allowed") open.push(item.to);
    expect(open).toEqual(["/ops/fca", "/ops/idst"]);
  });

  it("leaves a shared dashboard link open to any signed-in member", () => {
    const route = router.routesByPath["/ops/my/shared/$slug" as keyof typeof router.routesByPath];
    expect(route).toBeDefined();
    expect((route.options as {beforeLoad?: unknown}).beforeLoad).toBeUndefined();
  });

  it("refuses to build a guard for a path with no nav item", () => {
    expect(() => navGuard("/ops/nowhere")).toThrow(/no nav item/);
  });
});

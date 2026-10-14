// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import {ThemeProvider, ToastProvider, TooltipProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {Me} from "@/lib/auth";

/**
 * The Admin area's sidebar after the one-item "Flow" group was folded into "Admin" (#822). The real
 * sidebar renders under a real TanStack router at `/admin/flow/sectors`, so the active highlight is
 * the one the router's `Link` sets, not a value this test computes.
 */
const auth = vi.hoisted(() => ({me: null as Me | null}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: auth.me}),
  useLogout: () => ({mutate: () => undefined}),
  useLogin: () => ({mutate: () => undefined}),
  useSignInPending: () => false,
}));

import {AppSidebar} from "./app-sidebar";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // The theme toggle reads the OS colour scheme, and jsdom has no `matchMedia`.
  window.matchMedia ??= ((query: string) =>
    ({matches: false, media: query, addEventListener() {}, removeEventListener() {}}) as unknown as MediaQueryList);
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

function user(permissions: Record<string, unknown>, server_admin = false): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin,
    role_names: [],
    tmu_national: false,
    permissions: permissions as Me["permissions"],
  } as Me;
}

async function renderAt(path: string, me: Me): Promise<HTMLElement> {
  auth.me = me;
  const rootRoute = createRootRoute({
    component: () => (
      <ThemeProvider>
        <ToastProvider>
          <TooltipProvider>
            <AppSidebar collapsed={false} onToggle={() => undefined} />
          </TooltipProvider>
        </ToastProvider>
      </ThemeProvider>
    ),
  });
  const splat = createRoute({getParentRoute: () => rootRoute, path: "$"});
  const router = createRouter({
    routeTree: rootRoute.addChildren([splat]),
    history: createMemoryHistory({initialEntries: [path]}),
  });
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    await router.load();
    root!.render(<RouterProvider router={router} />);
  });
  return host;
}

/** Each sidebar group heading, with the link labels under it. */
function groups(host: HTMLElement): {heading: string; links: string[]}[] {
  return [...host.querySelectorAll("div.uppercase")].map((label) => ({
    heading: (label.textContent ?? "").trim(),
    links: [...label.parentElement!.querySelectorAll("a")].map((a) => (a.textContent ?? "").trim()),
  }));
}

/** The labels of every link the router marked as the current page. */
function active(host: HTMLElement): string[] {
  return [...host.querySelectorAll('a[aria-current="page"], a[data-status="active"]')].map((a) =>
    (a.textContent ?? "").trim(),
  );
}

describe("the Admin area's sidebar at /admin/flow/sectors (#822)", () => {
  it("shows Sectors last under Admin with no Flow heading, highlighted, for a server admin", async () => {
    const host = await renderAt("/admin/flow/sectors", user({}, true));
    const headings = groups(host).map((g) => g.heading);
    expect(headings).toContain("Admin");
    expect(headings).not.toContain("Flow");
    expect(groups(host).find((g) => g.heading === "Admin")!.links.at(-1)).toBe("Sectors");
    expect(active(host)).toEqual(["Sectors"]);
  });

  it("gives a flow.sectors.read-only holder an Admin group of Overview then Sectors, Sectors highlighted", async () => {
    const host = await renderAt("/admin/flow/sectors", user({flow: {sectors: ["read"]}}));
    expect(groups(host).map((g) => g.heading)).not.toContain("Flow");
    expect(groups(host).find((g) => g.heading === "Admin")?.links).toEqual(["Overview", "Sectors"]);
    expect(active(host)).toEqual(["Sectors"]);
  });

  it("highlights Overview, not Sectors, on /admin", async () => {
    const host = await renderAt("/admin", user({flow: {sectors: ["read"]}}));
    expect(active(host)).toEqual(["Overview"]);
  });
});

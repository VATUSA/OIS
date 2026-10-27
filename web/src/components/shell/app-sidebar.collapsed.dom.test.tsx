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
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ThemeProvider, ToastProvider, TooltipProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {Me} from "@/lib/auth";

// Coverage matrix (VATUSA/OIS#823): the collapsed sidebar's group flyouts in the real app sidebar
//
// Requirement                                    | Test                                                                          | Status
// -----------------------------------------------|-------------------------------------------------------------------------------|-------
// AC1 hidden groups not rendered (no perms)      | gives a member with no permissions only the Home link and the Advisories …    | done
// AC1 a group appears with its first permission  | adds an Operations trigger for a flow.fca.read holder …                       | done
// AC1 Planning without Admin/Historical          | gives an events.plan.read holder a Planning trigger and no Historical …       | done
// AC1 all seven, in order                        | shows Home and the six group triggers in order …                              | done
// AC1 signed out                                 | shows the Advisories trigger and no User trigger when signed out              | done
// AC2 + AC3 aria-current in a real flyout        | marks Events aria-current="page" in the Planning flyout on …                  | done
// AC2 navigation via an item closes + moves      | closes the flyout and moves the active trigger when a flyout item is …        | done
// AC2 navigation by other means closes           | closes an open flyout when the route changes without a flyout selection       | done
// AC4 active group per route (each group)        | marks exactly the {group} trigger active on {path}                            | done
// AC4 Admin landing exact                        | marks the Admin trigger active on /admin and not on /admin/planning/events    | done
// AC4 deep page under a group                    | keeps the Planning trigger active on a page below Events …                    | done
// AC4 non-exact item, no group prefix           | marks the Advisories trigger active on a page below Facility Map …            | done
// AC4 every User link                            | marks exactly the User trigger active on {path} for an api_keys.key.create …  | done
// AC4 API keys link only when shown              | marks no trigger active on /api-keys for a member who cannot create keys      | done
// AC4 Home                                       | marks the Home link current and no trigger active on /                        | done
// Operator: User flyout contents                 | lists Profile, Settings, Desktop app, What's new and Sign out …               | done
// Operator: API keys item gated                  | adds API keys to the User flyout only for an api_keys.key.create holder       | done
// Operator: Sign out arms in place               | keeps the User flyout open after Sign out's first click, and signs out on …   | done
// Operator: What's new                           | closes the User flyout when What's new is selected                            | done
// AC6 expanded unchanged                         | renders no flyout triggers in the expanded sidebar, with every visible …      | done
// AC7 phone drawer unchanged                     | renders every visible link inline in the phone drawer, with no flyout …       | done
//
// A real TanStack memory router (root route + `$` splat), so `aria-current` and the active trigger
// come from the router, not this test. Every principal is `server_admin: false` with a real permission
// tree: `server_admin: true` passes every `hasPermission` and proves no gate.

const auth = vi.hoisted(() => ({me: null as Me | null, logout: vi.fn()}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: auth.me}),
  useLogout: () => ({mutate: auth.logout}),
  useLogin: () => ({mutate: () => undefined}),
  useSignInPending: () => false,
}));

// The build's release stage (#841); the real one comes from the repo-root STAGE file.
const build = vi.hoisted(() => ({stage: "alpha" as "alpha" | "beta" | "rc" | "stable"}));
vi.mock("@/lib/build-stage", () => ({
  get BUILD_STAGE() {
    return build.stage;
  },
}));

import {AppSidebar, MobileNavButton} from "./app-sidebar";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia ??= ((query: string) =>
    ({matches: false, media: query, addEventListener() {}, removeEventListener() {}}) as unknown as MediaQueryList);
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.scrollIntoView ??= () => {};
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  document.body.innerHTML = "";
  auth.logout.mockClear();
  build.stage = "alpha";
});

/** A `Me` holding exactly `permissions` (dotted names), never `server_admin`. */
function user(...permissions: string[]): Me {
  const tree: Record<string, unknown> = {};
  for (const p of permissions) {
    const parts = p.split(".");
    const action = parts.pop()!;
    let node = tree;
    for (const seg of parts.slice(0, -1)) node = (node[seg] ??= {}) as Record<string, unknown>;
    const leaf = parts.at(-1)!;
    node[leaf] = [...((node[leaf] as string[] | undefined) ?? []), action];
  }
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: false,
    role_names: [],
    tmu_national: false,
    permissions: tree as Me["permissions"],
  } as Me;
}

const ADMIN = ["events.plan.read", "stats.data.read", "access.users.read"];

type Mode = "collapsed" | "expanded" | "drawer";

async function renderAt(path: string, me: Me | null, mode: Mode = "collapsed") {
  auth.me = me;
  const qc = new QueryClient({defaultOptions: {queries: {retry: false, enabled: false}}});
  const rootRoute = createRootRoute({
    component: () => (
      <QueryClientProvider client={qc}>
        <ThemeProvider>
          <ToastProvider>
            <TooltipProvider>
              {mode === "drawer" ? (
                <MobileNavButton />
              ) : (
                <AppSidebar collapsed={mode === "collapsed"} onToggle={() => undefined} />
              )}
            </TooltipProvider>
          </ToastProvider>
        </ThemeProvider>
      </QueryClientProvider>
    ),
  });
  const splat = createRoute({getParentRoute: () => rootRoute, path: "$"});
  const router = createRouter({
    routeTree: rootRoute.addChildren([splat]),
    history: createMemoryHistory({initialEntries: [path]}),
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    await router.load();
    root!.render(<RouterProvider router={router} />);
  });
  return {host, router};
}

/** The group triggers in the sidebar's nav (the chrome row's Recent pages menu is not one). */
const triggers = () => [...document.querySelectorAll<HTMLButtonElement>("aside nav button[aria-haspopup]")];
const triggerNames = () => triggers().map((t) => t.textContent);
const trigger = (name: string) => triggers().find((t) => t.textContent === name)!;
const activeTriggers = () => triggers().filter((t) => t.hasAttribute("data-active")).map((t) => t.textContent);
const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
const menuItems = () => [...(menu()?.querySelectorAll('[role="menuitem"]') ?? [])].map((i) => (i.textContent ?? "").trim());
const tick = () => act(async () => new Promise((r) => setTimeout(r, 0)));

async function open(name: string) {
  act(() => {
    trigger(name).dispatchEvent(new MouseEvent("pointerdown", {bubbles: true, cancelable: true, button: 0}));
  });
  await tick();
  expect(menu(), `${name} flyout opened`).not.toBeNull();
}

describe("collapsed AppSidebar group triggers (#823)", () => {
  it("gives a member with no permissions only the Home link and the Advisories and User triggers, with no Operations, Planning, Historical or Admin trigger", async () => {
    await renderAt("/advisories", user());
    expect(triggerNames()).toEqual(["Advisories", "User"]);
    expect(document.querySelector('aside nav a[href="/"]')).not.toBeNull();
  });

  it("adds an Operations trigger for a flow.fca.read holder whose flyout lists exactly TMU, FCA and IDST", async () => {
    await renderAt("/advisories", user("flow.fca.read"));
    expect(triggerNames()).toEqual(["Advisories", "Operations", "User"]);
    await open("Operations");
    expect(menuItems()).toEqual(["TMU", "FCA", "IDST"]);
  });

  it("gives an events.plan.read holder a Planning trigger and no Historical or Admin trigger", async () => {
    await renderAt("/advisories", user("events.plan.read"));
    expect(triggerNames()).toEqual(["Advisories", "Planning", "User"]);
  });

  it("shows Home and the Advisories, Operations, Planning, Historical, Admin and User triggers in that order for a holder of a permission in every group", async () => {
    await renderAt("/advisories", user("tmu.program.read", ...ADMIN));
    expect(triggerNames()).toEqual(["Advisories", "Operations", "Planning", "Historical", "Admin", "User"]);
    const nav = document.querySelector("aside nav")!;
    expect(nav.firstElementChild!.querySelector('a[href="/"]')).not.toBeNull();
  });

  it("shows the Advisories trigger and no User trigger when signed out", async () => {
    await renderAt("/advisories", null);
    expect(triggerNames()).toEqual(["Advisories"]);
  });
});

describe("collapsed AppSidebar active group (#823)", () => {
  const cases: [string, string][] = [
    ["Advisories", "/advisories/fcas"],
    ["Operations", "/ops/airport"],
    ["Planning", "/admin/planning/airport-configs"],
    ["Historical", "/admin/historical/replay"],
    ["Admin", "/admin/access"],
    ["User", "/profile"],
  ];
  for (const [group, path] of cases) {
    it(`marks exactly the ${group} trigger active on ${path}`, async () => {
      await renderAt(path, user("tmu.program.read", ...ADMIN));
      expect(activeTriggers()).toEqual([group]);
    });
  }

  it("marks the Advisories trigger active on a page below Facility Map, which no nav item names", async () => {
    await renderAt("/facility-map/ZNY", user(...ADMIN));
    expect(activeTriggers()).toEqual(["Advisories"]);
  });

  for (const path of ["/profile", "/settings", "/api-keys", "/download"]) {
    it(`marks exactly the User trigger active on ${path} for an api_keys.key.create holder`, async () => {
      await renderAt(path, user("api_keys.key.create", ...ADMIN));
      expect(activeTriggers()).toEqual(["User"]);
    });
  }

  it("marks no trigger active on /api-keys for a member who cannot create keys", async () => {
    await renderAt("/api-keys", user(...ADMIN));
    expect(activeTriggers()).toEqual([]);
  });

  it("marks the Admin trigger active on /admin and not on /admin/planning/events", async () => {
    await renderAt("/admin", user(...ADMIN));
    expect(activeTriggers()).toEqual(["Admin"]);
    act(() => root!.unmount());
    root = undefined;
    document.body.innerHTML = "";
    await renderAt("/admin/planning/events", user(...ADMIN));
    expect(activeTriggers()).toEqual(["Planning"]);
  });

  it("keeps the Planning trigger active on a page below Events, and on a Planning page that is no nav item", async () => {
    await renderAt("/admin/planning/events/42", user(...ADMIN));
    expect(activeTriggers()).toEqual(["Planning"]);
    act(() => root!.unmount());
    root = undefined;
    document.body.innerHTML = "";
    await renderAt("/admin/planning", user(...ADMIN));
    expect(activeTriggers()).toEqual(["Planning"]);
  });

  it('marks the Home link aria-current="page" and no trigger active on /', async () => {
    await renderAt("/", user(...ADMIN));
    expect(document.querySelector('aside nav a[href="/"]')!.getAttribute("aria-current")).toBe("page");
    expect(activeTriggers()).toEqual([]);
  });
});

describe("collapsed AppSidebar flyout navigation (#823)", () => {
  it('marks Events aria-current="page" in the Planning flyout on /admin/planning/events, and no other flyout item', async () => {
    await renderAt("/admin/planning/events", user(...ADMIN));
    await open("Planning");
    const current = [...menu()!.querySelectorAll('[aria-current="page"]')].map((a) => (a.textContent ?? "").trim());
    expect(current).toEqual(["Events"]);
    expect(menuItems()).toContain("Airport Configs");
  });

  it("closes the flyout and moves the active trigger when a flyout item is selected", async () => {
    const {router} = await renderAt("/advisories", user("flow.fca.read"));
    expect(activeTriggers()).toEqual(["Advisories"]);
    await open("Operations");
    const fca = [...menu()!.querySelectorAll<HTMLAnchorElement>("a")].find((a) => a.textContent?.trim() === "FCA")!;
    await act(async () => fca.click());
    await tick();
    expect(router.state.location.pathname).toBe("/ops/fca");
    expect(menu()).toBeNull();
    expect(activeTriggers()).toEqual(["Operations"]);
  });

  it("closes an open flyout when the route changes without a flyout selection", async () => {
    const {router} = await renderAt("/advisories", user("flow.fca.read"));
    await open("Operations");
    await act(async () => {
      await router.navigate({to: "/profile"});
    });
    await tick();
    expect(menu()).toBeNull();
    expect(activeTriggers()).toEqual(["User"]);
  });
});

describe("collapsed AppSidebar User flyout (#823)", () => {
  it("lists Profile, Settings, Desktop app, What's new and Sign out in the User flyout, in that order", async () => {
    await renderAt("/advisories", user());
    await open("User");
    expect(menuItems()).toEqual(["Profile", "Settings", "Desktop app", "What's new", "Sign out"]);
  });

  it("adds API keys to the User flyout only for an api_keys.key.create holder", async () => {
    await renderAt("/advisories", user("api_keys.key.create"));
    await open("User");
    expect(menuItems()).toEqual(["Profile", "Settings", "API keys", "Desktop app", "What's new", "Sign out"]);
  });

  it("keeps the User flyout open after Sign out's first click, and signs out on the second", async () => {
    await renderAt("/advisories", user());
    await open("User");
    const signOut = () =>
      [...menu()!.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === "Sign out")!;
    act(() => signOut().click());
    await tick();
    expect(menu()).not.toBeNull();
    expect(auth.logout).not.toHaveBeenCalled();
    act(() => signOut().click());
    expect(auth.logout).toHaveBeenCalledTimes(1);
  });

  it("closes the User flyout when What's new is selected", async () => {
    await renderAt("/advisories", user());
    await open("User");
    const opened = vi.fn();
    window.addEventListener("ois:whats-new", opened);
    const whatsNew = [...menu()!.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === "What's new",
    )!;
    act(() => whatsNew.click());
    await tick();
    window.removeEventListener("ois:whats-new", opened);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(menu()).toBeNull();
  });
});

describe("AppSidebar outside collapsed mode (#823)", () => {
  it("renders no flyout triggers in the expanded sidebar, with every visible link inline under its heading", async () => {
    await renderAt("/admin/planning/events", user(...ADMIN), "expanded");
    expect(triggers()).toEqual([]);
    const headings = [...document.querySelectorAll("aside nav div.uppercase")].map((h) => h.textContent);
    expect(headings).toEqual(["Advisories", "Planning", "Historical", "Admin", "User"]);
    expect(document.querySelector('aside nav a[href="/admin/planning/events"]')!.getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("renders every visible link inline in the phone drawer, with no flyout triggers", async () => {
    await renderAt("/admin/planning/events", user(...ADMIN), "drawer");
    act(() => document.querySelector<HTMLButtonElement>('button[aria-label="Menu"]')!.click());
    expect(document.querySelectorAll("button[aria-haspopup]")).toHaveLength(0);
    expect(document.querySelector('a[href="/admin/planning/events"]')).not.toBeNull();
    expect(document.querySelector('a[href="/profile"]')).not.toBeNull();
  });
});

// The release-stage chip in the sidebar header (#841). Its own label, tones and hover text are
// pinned in release-stage-chip.dom.test.tsx; these pin where the shell puts it.
describe("AppSidebar release stage chip (#841)", () => {
  const chip = (scope: ParentNode = document) => scope.querySelector<HTMLElement>("[data-release-stage]");
  const stageRow = () => document.querySelector<HTMLElement>("aside [data-stage-row]");

  it("shows the OIS wordmark and the ALPHA chip in the expanded header, in the row directly below the chrome row", async () => {
    await renderAt("/advisories", user(), "expanded");
    const row = stageRow()!;
    expect(row).not.toBeNull();
    expect(row.textContent).toBe("OISAlpha");
    expect(chip(row)!.textContent).toBe("Alpha");
    expect(chip(row)!.firstElementChild!.className).toContain("uppercase");
    expect(row.previousElementSibling!.querySelector('button[aria-label="Back"]')).not.toBeNull();
  });

  it("keeps the chrome row as the header's first child, with the stage row as its next sibling", async () => {
    await renderAt("/advisories", user(), "expanded");
    const header = stageRow()!.parentElement!;
    expect(header.firstElementChild!.querySelector('button[aria-label="Collapse sidebar"]')).not.toBeNull();
    expect(header.children[1]).toBe(stageRow());
  });

  it("shows the compact chip centered in the collapsed rail, with its hover title and no wordmark", async () => {
    await renderAt("/advisories", user(), "collapsed");
    const row = stageRow()!;
    expect(row.className).toContain("justify-center");
    expect(row.textContent).toBe("Alpha");
    expect(chip(row)!.getAttribute("title")).toBe("Alpha: expect rough edges and frequent changes");
    expect(chip(row)!.firstElementChild!.className).toContain("text-[10px]");
    expect(row.previousElementSibling!.querySelector('button[aria-label="Expand sidebar"]')).not.toBeNull();
  });

  it('shows "OIS" and the chip in the phone drawer title, and the dialog\'s aria-label is still "OIS"', async () => {
    await renderAt("/advisories", user(), "drawer");
    act(() => document.querySelector<HTMLButtonElement>('button[aria-label="Menu"]')!.click());
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.getAttribute("aria-label")).toBe("OIS");
    const title = dialog.querySelector("h2")!;
    expect(title.textContent).toBe("OISAlpha");
    expect(chip(title)).not.toBeNull();
  });

  // The header does not vary by route (AppShell renders this one sidebar on every signed-in page),
  // so one page outside Advisories is enough to show it is not tied to the route above.
  it("shows the chip in the header on /admin/access for a member with permissions", async () => {
    await renderAt("/admin/access", user("flow.fca.read", "access.users.read"), "expanded");
    expect(chip(stageRow()!)!.textContent).toBe("Alpha");
  });

  it("keeps the wordmark and drops the chip in the expanded header when the stage is stable", async () => {
    build.stage = "stable";
    await renderAt("/advisories", user(), "expanded");
    expect(stageRow()!.textContent).toBe("OIS");
    expect(chip()).toBeNull();
  });

  it("hides the empty stage row in the collapsed rail when the stage is stable", async () => {
    build.stage = "stable";
    await renderAt("/advisories", user(), "collapsed");
    expect(stageRow()!.childElementCount).toBe(0);
    expect(stageRow()!.className).toContain("empty:hidden");
    expect(chip()).toBeNull();
  });
});

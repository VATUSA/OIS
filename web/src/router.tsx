import {createRootRoute, createRoute, createRouter, Outlet, redirect, useRouterState,} from "@tanstack/react-router";

import {FeedWatcher} from "@/components/feed-watcher";
import {Navbar} from "@/components/navbar";
import {AdvisoriesPage} from "@/pages/advisories";
import {AdvisoriesFcaPage} from "@/pages/advisories/fcas";
import {PilotPage} from "@/pages/pilot";
import {ProfilePage} from "@/pages/profile";
import {AirportPage} from "@/pages/airport";
import {FcaPage} from "@/pages/fca";
import {RunwayPage} from "@/pages/runway";
import {DashboardPage} from "@/pages/dashboard";
import {BoardViewPage} from "@/pages/dashboards/board";
import {BoardLibraryPage} from "@/pages/dashboards/library";
import {SharedBoardPage} from "@/pages/dashboards/shared";
import {TmuPage} from "@/pages/tmu";
import {PlanningEventsPage} from "@/pages/planning/events";
import {EventPlanningPage} from "@/pages/planning/event";
import {AdminLayout} from "@/pages/admin/layout";
import {AdminOverview} from "@/pages/admin/overview";
import {AdminAccessControl} from "@/pages/admin/access-control";
import {AdminAudit} from "@/pages/admin/audit";
import {AdminServiceAccounts} from "@/pages/admin/service-accounts";

function RootLayout() {
  // Full-bleed routes (the FCA map) escape the centered, padded main wrapper.
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const fullBleed =
    pathname.startsWith("/ops/fca") ||
    pathname.startsWith("/ops/runway") ||
    pathname.startsWith("/advisories/fcas");
  return (
    <div className="min-h-screen bg-background text-foreground">
      <FeedWatcher />
      <Navbar />
      {fullBleed ? (
        <Outlet />
      ) : (
        <main className="mx-auto w-full max-w-7xl px-4 py-8">
          <Outlet />
        </main>
      )}
    </div>
  );
}

const rootRoute = createRootRoute({ component: RootLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

// --- Operations (live, during-event) ---

const opsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "ops",
  component: Outlet,
});

const opsIndexRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/ops/airport" });
  },
});

const airportRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "airport",
  component: AirportPage,
});

const tmuRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "tmu",
  component: TmuPage,
});

// Dashboards: a library at /ops/my, a board at /ops/my/$boardId, a shared read-only view at
// /ops/my/shared/$slug. Static "shared" wins over "$boardId" in TanStack's match ordering.
const myRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "my",
  component: Outlet,
});
const myIndexRoute = createRoute({
  getParentRoute: () => myRoute,
  path: "/",
  component: BoardLibraryPage,
});
const sharedBoardRoute = createRoute({
  getParentRoute: () => myRoute,
  path: "shared/$slug",
  component: SharedBoardPage,
});
const boardRoute = createRoute({
  getParentRoute: () => myRoute,
  path: "$boardId",
  component: BoardViewPage,
});

const fcaRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "fca",
  component: FcaPage,
});

const runwayRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "runway",
  component: RunwayPage,
});

// --- Advisories (public, read-only) ---

const advisoriesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "advisories",
  component: Outlet,
});

const advisoriesIndexRoute = createRoute({
  getParentRoute: () => advisoriesRoute,
  path: "/",
  component: AdvisoriesPage,
});

const advisoriesFcaRoute = createRoute({
  getParentRoute: () => advisoriesRoute,
  path: "fcas",
  component: AdvisoriesFcaPage,
  validateSearch: (search: Record<string, unknown>): { flight?: string } => ({
    flight: typeof search.flight === "string" ? search.flight : undefined,
  }),
});

// Pilot "my flight" lookup — public, its own top-level route.
const pilotRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "pilot",
  component: PilotPage,
});

const profileRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "profile",
  component: ProfilePage,
});

// --- Planning (pre-event) ---

const planningRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "planning",
  component: Outlet,
});

const planningIndexRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/planning/events" });
  },
});

const planningEventsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events",
  component: PlanningEventsPage,
});

const planningEventRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId",
  component: EventPlanningPage,
});

// --- Admin ---

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin",
  component: AdminLayout,
});

const adminIndexRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "/",
  component: AdminOverview,
});

const adminAccessRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "access",
  component: AdminAccessControl,
});

const adminAuditRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "audit",
  component: AdminAudit,
});

const adminServiceAccountsRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "service-accounts",
  component: AdminServiceAccounts,
});

// --- Legacy path redirects (old flat routes → /ops/*) ---

const legacyRedirects = (
  [
    ["/airport", "/ops/airport"],
    // Departures + Taxi were merged into the Airport page (tabs).
    ["/departures", "/ops/airport"],
    ["/taxi", "/ops/airport"],
    ["/ops/departures", "/ops/airport"],
    ["/ops/taxi", "/ops/airport"],
    ["/tmu", "/ops/tmu"],
    ["/my", "/ops/my"],
  ] as const
).map(([from, to]) =>
  createRoute({
    getParentRoute: () => rootRoute,
    path: from,
    beforeLoad: () => {
      throw redirect({ to });
    },
  }),
);

const routeTree = rootRoute.addChildren([
  indexRoute,
  opsRoute.addChildren([
    opsIndexRoute,
    airportRoute,
    tmuRoute,
    myRoute.addChildren([myIndexRoute, sharedBoardRoute, boardRoute]),
    fcaRoute,
    runwayRoute,
  ]),
  advisoriesRoute.addChildren([advisoriesIndexRoute, advisoriesFcaRoute]),
  pilotRoute,
  profileRoute,
  planningRoute.addChildren([
    planningIndexRoute,
    planningEventsRoute,
    planningEventRoute,
  ]),
  adminRoute.addChildren([
    adminIndexRoute,
    adminAccessRoute,
    adminAuditRoute,
    adminServiceAccountsRoute,
  ]),
  ...legacyRedirects,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

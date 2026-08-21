import {createRootRoute, createRoute, createRouter, lazyRouteComponent, Outlet, redirect, useRouterState,} from "@tanstack/react-router";

import {FeedWatcher} from "@/components/feed-watcher";
import {Footer} from "@/components/footer";
import {Navbar} from "@/components/navbar";
import {AdvisoriesPage} from "@/pages/advisories";
import {AdvisoriesFcaPage} from "@/pages/advisories/fcas";
import {PilotPage} from "@/pages/pilot";
import {PrivacyPage} from "@/pages/privacy";
import {ProfilePage} from "@/pages/profile";
import {SettingsPage} from "@/pages/settings";
import {AirportPage} from "@/pages/airport";
import {FcaPage} from "@/pages/fca";
import {FacilityMapIndexPage, FacilityMapPage} from "@/pages/facility-map";
import {RunwayPage} from "@/pages/runway";
import {DashboardPage} from "@/pages/dashboard";
import {BoardViewPage} from "@/pages/dashboards/board";
import {BoardLibraryPage} from "@/pages/dashboards/library";
import {SharedBoardPage} from "@/pages/dashboards/shared";
import {TmuPage} from "@/pages/tmu";
import {PlanningEventsPage} from "@/pages/planning/events";
import {EventPlanningPage} from "@/pages/planning/event";
import {AirportConfigsPage} from "@/pages/planning/airport-configs";
import {StatsPage} from "@/pages/stats";
import {StatsFlightPage} from "@/pages/stats/flight";
// Replay pulls in deck.gl + MapLibre — code-split so it only loads on its route.
const CaptureReplayPage = lazyRouteComponent(() => import("@/pages/stats/replay"), "CaptureReplayPage");
const HistoricalDashboardPage = lazyRouteComponent(
  () => import("@/pages/stats/dashboard"),
  "HistoricalDashboardPage",
);
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
    pathname.startsWith("/advisories/fcas") ||
    pathname.startsWith("/facility-map");
  return (
    <div className="flex min-h-screen flex-col bg-background text-foreground">
      <FeedWatcher />
      <Navbar />
      {fullBleed ? (
        <Outlet />
      ) : (
        <>
          <main className="mx-auto w-full max-w-7xl flex-1 px-4 py-8">
            <Outlet />
          </main>
          <Footer />
        </>
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

// Facility map — public per-facility TMU map. Landing (picker) + full-bleed `$facilityId` map.
const facilityMapRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "facility-map",
  component: Outlet,
});
const facilityMapIndexRoute = createRoute({
  getParentRoute: () => facilityMapRoute,
  path: "/",
  component: FacilityMapIndexPage,
});
const facilityMapDetailRoute = createRoute({
  getParentRoute: () => facilityMapRoute,
  path: "$facilityId",
  component: FacilityMapPage,
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

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "settings",
  component: SettingsPage,
});

// Public legal/info pages (linked from the footer).
const privacyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "privacy",
  component: PrivacyPage,
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

const planningAirportConfigsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "airport-configs",
  component: AirportConfigsPage,
});

const planningEventRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId",
  component: EventPlanningPage,
});

// --- Historical (persisted network statistics, replay + dashboard) ---

const statsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "historical",
  component: Outlet,
});

const statsIndexRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "/",
  component: StatsPage,
});

const statsFlightRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "flights/$flightId",
  component: StatsFlightPage,
});

// Standalone replay map: pick a saved capture OR a custom [from, to] window, then scrub. The old
// per-capture deep link is preserved as `?capture=<id>`.
const statsReplayRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "replay",
  component: CaptureReplayPage,
  validateSearch: (
    search: Record<string, unknown>,
  ): { capture?: string; from?: number; to?: number; step?: number } => {
    const num = (v: unknown) => (v != null && Number.isFinite(Number(v)) ? Number(v) : undefined);
    return {
      capture: typeof search.capture === "string" ? search.capture : undefined,
      from: num(search.from),
      to: num(search.to),
      step: num(search.step),
    };
  },
});

const statsDashboardRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "dashboard",
  component: HistoricalDashboardPage,
  // Deep-link a replay: a capture (or a custom from/to window), a board, and the scrubber instant.
  validateSearch: (
    search: Record<string, unknown>,
  ): { capture?: string; from?: number; to?: number; board?: string; t?: number } => {
    const num = (v: unknown) => (v != null && Number.isFinite(Number(v)) ? Number(v) : undefined);
    return {
      capture: typeof search.capture === "string" ? search.capture : undefined,
      from: num(search.from),
      to: num(search.to),
      board: typeof search.board === "string" ? search.board : undefined,
      t: num(search.t),
    };
  },
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
  facilityMapRoute.addChildren([facilityMapIndexRoute, facilityMapDetailRoute]),
  pilotRoute,
  profileRoute,
  settingsRoute,
  privacyRoute,
  planningRoute.addChildren([
    planningIndexRoute,
    planningEventsRoute,
    planningAirportConfigsRoute,
    planningEventRoute,
  ]),
  statsRoute.addChildren([statsIndexRoute, statsFlightRoute, statsReplayRoute, statsDashboardRoute]),
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

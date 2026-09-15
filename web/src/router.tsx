import {createRootRoute, createRoute, createRouter, lazyRouteComponent, Outlet, redirect, useRouterState,} from "@tanstack/react-router";

import {FeedWatcher} from "@/components/feed-watcher";
import {RestrictionAlerts} from "@/components/restriction-alerts";
import {Footer} from "@/components/footer";
import {Navbar} from "@/components/navbar";
import {useMe} from "@/lib/auth";
import {AdvisoriesPage} from "@/pages/advisories";
import {AdvisoriesFcaPage} from "@/pages/advisories/fcas";
import {PilotPage} from "@/pages/pilot";
import {PrivacyPage} from "@/pages/privacy";
import {ProfilePage} from "@/pages/profile";
import {SettingsPage} from "@/pages/settings";
import {ApiKeysPage} from "@/pages/api-keys";
import {AirportPage} from "@/pages/airport";
import {FcaPage} from "@/pages/fca";
import {IdstPage} from "@/pages/idst";
import {FacilityMapIndexPage, FacilityMapPage} from "@/pages/facility-map";
import {RunwayPage} from "@/pages/runway";
import {DashboardPage} from "@/pages/dashboard";
import {BoardViewPage} from "@/pages/dashboards/board";
import {BoardLibraryPage} from "@/pages/dashboards/library";
import {SharedBoardPage} from "@/pages/dashboards/shared";
import {TmuPage} from "@/pages/tmu";
import {PlanningEventsPage} from "@/pages/planning/events";
import {EventPlanningPage} from "@/pages/planning/event";
import {EventFcaBuilderPage} from "@/pages/planning/event-fcas";
import {AircraftProfilesPage} from "@/pages/planning/aircraft-profiles";
import {AirportConfigsPage} from "@/pages/planning/airport-configs";
import {AirportSurfacePage} from "@/pages/planning/airport-surface";
import {FacilityDocumentsPage} from "@/pages/planning/facility-documents";
import {StatsPage} from "@/pages/stats";
import {DelaysPage} from "@/pages/stats/delays";
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
import {AdminJobs} from "@/pages/admin/jobs";
import {AdminApiKeys} from "@/pages/admin/api-keys";
import {AdminDiscord} from "@/pages/admin/discord";

function RootLayout() {
  // A route can declare a width tier via `staticData.layout` (see the route definitions):
  //   "full" — no wrapper, owns the viewport (the maps);
  //   "wide" — full monitor width with padding, for data-dense pages (dashboards, replay, tables)
  //            so ultrawide displays aren't boxed into a narrow column;
  //   default — a readable centered column (forms, prose, detail views).
  // Read the deepest match that sets a layout so a group parent can set it for all its children.
  const layout = useRouterState({
    select: (s) => {
      for (let i = s.matches.length - 1; i >= 0; i--) {
        const l = s.matches[i].staticData?.layout;
        if (l) return l;
      }
      return undefined;
    },
  });
  // `?embed=1` on any route strips the whole app shell (no nav, footer, or feed watcher) so the page
  // fills an external `<iframe>`. The page itself reads the same param to show a minimal chrome.
  const embed = useRouterState({
    select: (s) => {
      const v = (s.location.search as Record<string, unknown> | undefined)?.embed;
      return v === true || v === 1 || v === "1" || v === "true";
    },
  });
  // `useMe` errors only when the backend is unreachable (a 401 resolves to `null`, not an error). In
  // that case every data-gated page would otherwise sit on "Loading…" forever, so show a clear
  // retrying state instead — the query keeps probing and recovers on its own when the API returns.
  const me = useMe();
  if (me.isError) {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background px-6 text-center text-foreground">
        <p className="text-lg font-semibold">Can’t reach OIS</p>
        <p className="max-w-md text-sm text-muted-foreground">
          The server isn’t responding right now. This page keeps trying and will reconnect
          automatically.
        </p>
        <button
          type="button"
          onClick={() => me.refetch()}
          className="rounded-md border px-4 py-2 text-sm transition-colors hover:bg-accent"
        >
          Retry now
        </button>
      </div>
    );
  }

  const mainClass =
    layout === "wide"
      ? "w-full flex-1 px-4 py-8 sm:px-6 2xl:px-10"
      : "mx-auto w-full max-w-7xl flex-1 px-4 py-8";
  // Keep `<Outlet>` at a stable child position across the embed/normal split — `embed` can flip from
  // false→true on the first render (search resolves a tick late), and if the Outlet moved between
  // branches React would remount the whole page (discarding e.g. a map's auto-fit).
  return (
    <div
      className={
        embed
          ? "h-[100dvh] w-full bg-background text-foreground"
          : "flex min-h-screen flex-col bg-background text-foreground"
      }
    >
      {embed ? null : <FeedWatcher />}
      {embed ? null : <RestrictionAlerts />}
      {embed ? null : <Navbar />}
      {embed || layout === "full" ? (
        <Outlet />
      ) : (
        <>
          <main className={mainClass}>
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
  staticData: { layout: "wide" },
});

const TMU_TAB_IDS = [
  "programs",
  "restrictions",
  "ground-stops",
  "gdp",
  "rate-calculator",
] as const;
type TmuTabId = (typeof TMU_TAB_IDS)[number];

const tmuRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "tmu",
  component: TmuPage,
  staticData: { layout: "wide" },
  // Which tab is active — permission-gated fallback (if the user can't see this tab) happens in
  // the component, since that depends on auth state this route-level validator doesn't have.
  validateSearch: (search: Record<string, unknown>): { tab?: TmuTabId } => ({
    tab: TMU_TAB_IDS.includes(search.tab as TmuTabId) ? (search.tab as TmuTabId) : undefined,
  }),
});

// Dashboards: a library at /ops/my, a board at /ops/my/$boardId, a shared read-only view at
// /ops/my/shared/$slug. Static "shared" wins over "$boardId" in TanStack's match ordering.
const myRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "my",
  component: Outlet,
  staticData: { layout: "wide" },
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
  staticData: { layout: "full" },
});

const runwayRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "runway",
  component: RunwayPage,
  staticData: { layout: "full" },
});

const idstRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "idst",
  component: IdstPage,
  staticData: { layout: "wide" },
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
  staticData: { layout: "full" },
  validateSearch: (search: Record<string, unknown>): { flight?: string } => ({
    flight: typeof search.flight === "string" ? search.flight : undefined,
  }),
});

// Facility map — public per-facility TMU map. Landing (picker) + full-bleed `$facilityId` map.
const facilityMapRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "facility-map",
  component: Outlet,
  staticData: { layout: "full" },
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
  // Embed controls: `?embed=1` → minimal chrome (map + aircraft + legend only); `atc`/`routes` turn
  // those layers on; `theme` forces light/dark for the host page.
  validateSearch: (
    search: Record<string, unknown>,
  ): {
    embed?: boolean;
    atc?: boolean;
    routes?: boolean;
    fixes?: boolean;
    theme?: "light" | "dark";
  } => {
    const bool = (v: unknown) => v === true || v === 1 || v === "1" || v === "true";
    const theme = search.theme === "light" || search.theme === "dark" ? search.theme : undefined;
    return {
      embed: bool(search.embed) || undefined,
      atc: bool(search.atc) || undefined,
      routes: bool(search.routes) || undefined,
      fixes: bool(search.fixes) || undefined,
      theme,
    };
  },
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

// Self-service API keys (personal access tokens) — account-level, its own top-level route.
const apiKeysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "api-keys",
  component: ApiKeysPage,
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
  staticData: { layout: "wide" },
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

const planningFacilityDocumentsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "facility-documents",
  component: FacilityDocumentsPage,
});

const planningAirportSurfaceRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "airport-surface",
  component: AirportSurfacePage,
});

const planningAircraftProfilesRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "aircraft-profiles",
  component: AircraftProfilesPage,
});

const planningEventRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId",
  component: EventPlanningPage,
});

const planningEventFcasRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId/fcas",
  component: EventFcaBuilderPage,
  staticData: { layout: "full" },
});

// --- Historical (persisted network statistics, replay + dashboard) ---

const statsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "historical",
  component: Outlet,
  staticData: { layout: "wide" },
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

const statsDelaysRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "delays",
  component: DelaysPage,
});

// --- Admin ---

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin",
  component: AdminLayout,
  staticData: { layout: "wide" },
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

const adminJobsRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "jobs",
  component: AdminJobs,
});


const adminApiKeysRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "api-keys",
  component: AdminApiKeys,
});

const adminDiscordRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "discord",
  component: AdminDiscord,
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
    idstRoute,
  ]),
  advisoriesRoute.addChildren([advisoriesIndexRoute, advisoriesFcaRoute]),
  facilityMapRoute.addChildren([facilityMapIndexRoute, facilityMapDetailRoute]),
  pilotRoute,
  profileRoute,
  settingsRoute,
  apiKeysRoute,
  privacyRoute,
  planningRoute.addChildren([
    planningIndexRoute,
    planningEventsRoute,
    planningAirportConfigsRoute,
    planningFacilityDocumentsRoute,
    planningAirportSurfaceRoute,
    planningAircraftProfilesRoute,
    planningEventRoute,
    planningEventFcasRoute,
  ]),
  statsRoute.addChildren([
    statsIndexRoute,
    statsFlightRoute,
    statsReplayRoute,
    statsDashboardRoute,
    statsDelaysRoute,
  ]),
  adminRoute.addChildren([
    adminIndexRoute,
    adminAccessRoute,
    adminAuditRoute,
    adminJobsRoute,
    adminApiKeysRoute,
    adminDiscordRoute,
  ]),
  ...legacyRedirects,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
  /** Per-route width tier read by RootLayout. Omit for the default readable column. */
  interface StaticDataRouteOption {
    layout?: "full" | "wide";
  }
}

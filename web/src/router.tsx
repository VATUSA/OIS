import {createRootRoute, createRoute, createRouter, lazyRouteComponent, Outlet, redirect, useRouterState,} from "@tanstack/react-router";
import {Button} from "@ois/ui";
import {LayoutGrid, List, Rows3} from "lucide-react";

import {FeedWatcher} from "@/components/feed-watcher";
import {AppShell} from "@/components/shell/app-shell";
import type {RouteMeta} from "@/components/shell/page-meta";
import {RestrictionAlerts} from "@/components/restriction-alerts";
import {NotificationClicks} from "@/components/notification-clicks";
import {DesktopNotifiers} from "@/components/desktop-notifiers";
import {DesktopTray} from "@/components/desktop-tray";
import {WhatsNew} from "@/components/whats-new";
import {useMe} from "@/lib/auth";
import {movedPath} from "@/lib/moved-paths";
import {AdvisoriesPage} from "@/pages/advisories";
import {AdvisoriesFcaPage} from "@/pages/advisories/fcas";
import {PilotPage} from "@/pages/pilot";
import {PrivacyPage} from "@/pages/privacy";
import {DownloadPage} from "@/pages/download";
import {PopoutFcaLadderPage, PopoutWidgetPage} from "@/pages/popout";
import {ProfilePage} from "@/pages/profile";
import {SettingsPage} from "@/pages/settings";
import {ApiKeysPage} from "@/pages/api-keys";
import {AirportPage} from "@/pages/airport";
import {FcaPage} from "@/pages/fca";
import {IdstPage} from "@/pages/idst";
import {FacilityMapIndexPage, FacilityMapPage} from "@/pages/facility-map";
import {RunwayPage} from "@/pages/runway";
import {AadcPage} from "@/pages/aadc";
import {DashboardPage} from "@/pages/dashboard";
import {LandingPage} from "@/pages/landing";
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
import {TaxiInsightsPage} from "@/pages/stats/taxi-insights";
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

const isTruthy = (v: unknown) => v === true || v === 1 || v === "1" || v === "true";

function RootLayout() {
  // `?embed=1` on any route strips the whole app shell (no nav, sidebar, or feed watcher) so the page
  // fills an external `<iframe>`. Read the URL directly as well as router state: router search can
  // resolve a tick late, and flipping embed after mount would remount the page into a different tree
  // (discarding e.g. a map's auto-fit).
  const embed = useRouterState({
    select: (s) => isTruthy((s.location.search as Record<string, unknown> | undefined)?.embed),
  }) || isTruthy(new URLSearchParams(window.location.search).get("embed"));
  // `useMe` errors only when the backend is unreachable (a 401 resolves to `null`, not an error). In
  // that case every data-gated page would otherwise sit on "Loading…" forever, so show a clear
  // retrying state instead — the query keeps probing and recovers on its own when the API returns.
  const me = useMe();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  if (me.isError) {
    return (
      <div className="flex min-h-dvh flex-col items-center justify-center gap-4 bg-ground px-6 text-center text-ink">
        <p className="text-xl font-bold">Can’t reach OIS</p>
        <p className="max-w-md text-sm text-ink-2">
          The server isn’t responding right now. This page keeps trying and will reconnect
          automatically.
        </p>
        <Button variant="secondary" onClick={() => me.refetch()}>
          Retry now
        </Button>
      </div>
    );
  }

  // Signed-out visitors land on the public homepage — no app shell, with the site footer.
  if (pathname === "/" && !me.isLoading && me.data === null) return <LandingPage />;

  if (embed) {
    return (
      <div className="h-dvh w-full bg-ground text-ink">
        <Outlet />
      </div>
    );
  }
  return (
    <>
      <FeedWatcher />
      <RestrictionAlerts />
      <NotificationClicks />
      <DesktopNotifiers />
      <DesktopTray />
      <WhatsNew />
      <AppShell>
        <Outlet />
      </AppShell>
    </>
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
  staticData: { layout: "wide", title: "Airport" },
  // `?icao=` deep-links an airport (the ⌘K search jumps here).
  validateSearch: (search: Record<string, unknown>): { icao?: string } => ({
    icao: typeof search.icao === "string" ? search.icao.toUpperCase() : undefined,
  }),
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
  staticData: { layout: "wide", title: "TMU", views: [{ value: "board", label: "Board", icon: LayoutGrid }, { value: "table", label: "Table", icon: Rows3 }] },
  // Which tab is active — permission-gated fallback (if the user can't see this tab) happens in
  // the component, since that depends on auth state this route-level validator doesn't have.
  // `?facility=` pre-filters the restrictions list (the ⌘K search jumps here for a TMI).
  validateSearch: (search: Record<string, unknown>): { tab?: TmuTabId; facility?: string } => ({
    tab: TMU_TAB_IDS.includes(search.tab as TmuTabId) ? (search.tab as TmuTabId) : undefined,
    facility: typeof search.facility === "string" ? search.facility.toUpperCase() : undefined,
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
  staticData: { title: "My dashboards", views: [{ value: "grid", label: "Grid", icon: LayoutGrid }, { value: "list", label: "List", icon: List }] },
  component: BoardLibraryPage,
});
const sharedBoardRoute = createRoute({
  getParentRoute: () => myRoute,
  path: "shared/$slug",
  staticData: { title: "Shared dashboard" },
  component: SharedBoardPage,
});
const boardRoute = createRoute({
  getParentRoute: () => myRoute,
  path: "$boardId",
  staticData: { title: "Dashboard" },
  component: BoardViewPage,
});

const fcaRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "fca",
  component: FcaPage,
  staticData: { layout: "full", title: "FCA flow" },
});

const runwayRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "runway",
  component: RunwayPage,
  staticData: { layout: "full", title: "Runway balancer" },
});

const idstRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "idst",
  component: IdstPage,
  staticData: { layout: "wide", title: "IDST" },
});

const aadcRoute = createRoute({
  getParentRoute: () => opsRoute,
  path: "aadc",
  component: AadcPage,
  staticData: { layout: "wide", title: "Arrival demand chart" },
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
  staticData: { title: "Advisories" },
  component: AdvisoriesPage,
});

const advisoriesFcaRoute = createRoute({
  getParentRoute: () => advisoriesRoute,
  path: "fcas",
  component: AdvisoriesFcaPage,
  staticData: { layout: "full", title: "FCAs" },
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
  staticData: { title: "Facility map" },
  component: FacilityMapIndexPage,
});
const facilityMapDetailRoute = createRoute({
  getParentRoute: () => facilityMapRoute,
  path: "$facilityId",
  staticData: { title: "Facility map" },
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
  staticData: { title: "Pilot" },
  component: PilotPage,
});

const profileRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "profile",
  staticData: { title: "Profile" },
  component: ProfilePage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "settings",
  staticData: { title: "Settings" },
  component: SettingsPage,
});

// Self-service API keys (personal access tokens) — account-level, its own top-level route.
const apiKeysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "api-keys",
  staticData: { title: "API keys" },
  component: ApiKeysPage,
});

// Pop-out mini-windows (#349). Opened by the desktop app with `?embed=1`, so RootLayout renders
// them without the shell — just the panel, filling a small always-on-top window. Not linked from
// anywhere in the UI; the pop-out button creates the window.
const popoutWidgetRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "popout/widget/$boardId/$widgetId",
  staticData: { layout: "full", title: "Panel" },
  component: PopoutWidgetPage,
});

const popoutFcaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "popout/fca/$fcaId",
  staticData: { layout: "full", title: "Metering" },
  component: PopoutFcaLadderPage,
});

// Public legal/info pages (linked from the footer).
const downloadRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "download",
  staticData: { title: "Download" },
  component: DownloadPage,
});

const privacyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "privacy",
  staticData: { title: "Privacy" },
  component: PrivacyPage,
});

// --- Admin ---

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "admin",
  component: AdminLayout,
  staticData: { layout: "wide" },
});

// --- Planning (pre-event) ---

const planningRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "planning",
  component: Outlet,
  staticData: { layout: "wide" },
});

const planningIndexRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/admin/planning/events" });
  },
});

const planningEventsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events",
  staticData: { title: "Events", views: [{ value: "table", label: "Table", icon: Rows3 }, { value: "board", label: "Board", icon: LayoutGrid }] },
  component: PlanningEventsPage,
});

const planningAirportConfigsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "airport-configs",
  staticData: { title: "Airport configs", views: [{ value: "grouped", label: "By airport", icon: LayoutGrid }, { value: "list", label: "List", icon: List }] },
  component: AirportConfigsPage,
  // `?icao=` opens that airport's configs straight away (the ⌘K search jumps here).
  validateSearch: (search: Record<string, unknown>): { icao?: string } => ({
    icao: typeof search.icao === "string" ? search.icao.toUpperCase() : undefined,
  }),
});

const planningFacilityDocumentsRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "facility-documents",
  staticData: { title: "Facility documents" },
  component: FacilityDocumentsPage,
});

const planningAirportSurfaceRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "airport-surface",
  staticData: { title: "Airport surface" },
  component: AirportSurfacePage,
  // `?icao=` opens that airport's surface editor straight away (the ⌘K search jumps here).
  validateSearch: (search: Record<string, unknown>): { icao?: string } => ({
    icao: typeof search.icao === "string" ? search.icao.toUpperCase() : undefined,
  }),
});

const planningAircraftProfilesRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "aircraft-profiles",
  staticData: { title: "Aircraft profiles" },
  component: AircraftProfilesPage,
});

const planningEventRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId",
  staticData: { title: "Event" },
  component: EventPlanningPage,
});

const planningEventFcasRoute = createRoute({
  getParentRoute: () => planningRoute,
  path: "events/$eventId/fcas",
  component: EventFcaBuilderPage,
  staticData: { layout: "full", title: "Event FCAs" },
});

// --- Historical (persisted network statistics, replay + dashboard) ---

const statsRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "historical",
  component: Outlet,
  staticData: { layout: "wide" },
});

const statsIndexRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "/",
  staticData: { title: "Network stats" },
  component: StatsPage,
});

const statsFlightRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "flights/$flightId",
  staticData: { title: "Flight" },
  component: StatsFlightPage,
});

// Standalone replay map: pick a saved capture OR a custom [from, to] window, then scrub. The old
// per-capture deep link is preserved as `?capture=<id>`.
const statsReplayRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "replay",
  staticData: { title: "Replay" },
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
  staticData: { title: "Dashboard replay" },
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
  staticData: { title: "Delays" },
  component: DelaysPage,
});

const statsTaxiRoute = createRoute({
  getParentRoute: () => statsRoute,
  path: "taxi",
  staticData: { title: "Taxi insights" },
  component: TaxiInsightsPage,
});

const adminIndexRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "/",
  staticData: { title: "Overview", subtitle: "Planning, historical data and server administration at a glance." },
  component: AdminOverview,
});

const adminAccessRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "access",
  staticData: { title: "Access" },
  component: AdminAccessControl,
});

const adminAuditRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "audit",
  staticData: { title: "Audit log" },
  component: AdminAudit,
});

const adminJobsRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "jobs",
  staticData: { title: "Background jobs" },
  component: AdminJobs,
});


const adminApiKeysRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "api-keys",
  staticData: { title: "API keys" },
  component: AdminApiKeys,
});

const adminDiscordRoute = createRoute({
  getParentRoute: () => adminRoute,
  path: "discord",
  staticData: { title: "Discord" },
  component: AdminDiscord,
});

// --- Legacy path redirects ---

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

const movedRedirects = ["planning", "historical"].flatMap((base) =>
  [base, `${base}/$`].map((path) =>
    createRoute({
      getParentRoute: () => rootRoute,
      path,
      beforeLoad: ({ location }) => {
        const href = movedPath(location.pathname, location.searchStr);
        if (href) throw redirect({ href });
      },
    }),
  ),
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
    aadcRoute,
  ]),
  advisoriesRoute.addChildren([advisoriesIndexRoute, advisoriesFcaRoute]),
  facilityMapRoute.addChildren([facilityMapIndexRoute, facilityMapDetailRoute]),
  pilotRoute,
  profileRoute,
  settingsRoute,
  apiKeysRoute,
  popoutWidgetRoute,
  popoutFcaRoute,
  downloadRoute,
  privacyRoute,
  adminRoute.addChildren([
    adminIndexRoute,
    adminAccessRoute,
    adminAuditRoute,
    adminJobsRoute,
    adminApiKeysRoute,
    adminDiscordRoute,
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
      statsTaxiRoute,
    ]),
  ]),
  ...legacyRedirects,
  ...movedRedirects,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
  /** Per-route shell meta read by the AppShell (width tier, title, subtitle, icon, views). */
  // An empty interface is the only way to merge RouteMeta into the router's declared type.
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  interface StaticDataRouteOption extends RouteMeta {}
}

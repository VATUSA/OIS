import {createRootRoute, createRoute, createRouter, Outlet,} from "@tanstack/react-router";

import {Navbar} from "@/components/navbar";
import {AirportPage} from "@/pages/airport";
import {DashboardPage} from "@/pages/dashboard";
import {DeparturesPage} from "@/pages/departures";
import {TmuPage} from "@/pages/tmu";
import {AdminLayout} from "@/pages/admin/layout";
import {AdminOverview} from "@/pages/admin/overview";
import {AdminAccessControl} from "@/pages/admin/access-control";
import {AdminAudit} from "@/pages/admin/audit";
import {AdminServiceAccounts} from "@/pages/admin/service-accounts";

const rootRoute = createRootRoute({
  component: () => (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />
      <main className="mx-auto w-full max-w-7xl px-4 py-8">
        <Outlet />
      </main>
    </div>
  ),
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

const airportRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/airport",
  component: AirportPage,
});

const departuresRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/departures",
  component: DeparturesPage,
});

const tmuRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tmu",
  component: TmuPage,
});

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

const routeTree = rootRoute.addChildren([
  indexRoute,
  airportRoute,
  departuresRoute,
  tmuRoute,
  adminRoute.addChildren([
    adminIndexRoute,
    adminAccessRoute,
    adminAuditRoute,
    adminServiceAccountsRoute,
  ]),
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

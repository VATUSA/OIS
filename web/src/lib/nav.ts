import {
  Activity,
  BarChart3,
  CalendarClock,
  FileText,
  Film,
  Gauge,
  KeyRound,
  LayoutDashboard,
  type LucideIcon,
  MapPinned,
  Megaphone,
  MessageSquare,
  Plane,
  PlaneTakeoff,
  Radar,
  ScrollText,
  ShieldCheck,
  Split,
  Timer,
  TrendingUp,
  Waypoints,
  Wind,
} from "lucide-react";

import type {Me} from "./auth";
import {hasPermission} from "./permissions";

/**
 * The one navigation map. The top nav, the area sidebars, the mobile menu, breadcrumbs and the ⌘K
 * page search all derive from it, so a link's permission is declared once. An item is visible when
 * the user holds `permission` (or any of `anyOf`); an item with neither is public.
 */
export type NavItem = {
  label: string;
  to: string;
  icon: LucideIcon;
  permission?: string;
  anyOf?: readonly string[];
  /** Match only the exact path for the active state (an area's landing). */
  exact?: boolean;
};

/** `prefix` claims deeper pages under the group that aren't nav items (an event, a flight). */
export type NavGroup = { label?: string; prefix?: string; items: readonly NavItem[] };

export type NavArea = {
  id: "advisories" | "operations" | "admin";
  label: string;
  /** Pathname prefixes that belong to this area (drives the sidebar + breadcrumbs). */
  prefixes: readonly string[];
  groups: readonly NavGroup[];
};

const OPS_ANY = ["tmu.program.read", "tmu.tmi.read", "flow.fca.read", "flow.runway.read"] as const;

export const AREAS: readonly NavArea[] = [
  {
    id: "advisories",
    label: "Advisories",
    prefixes: ["/advisories", "/facility-map", "/pilot"],
    groups: [
      {
        items: [
          { label: "Advisories", to: "/advisories", icon: Megaphone, exact: true },
          { label: "FCAs", to: "/advisories/fcas", icon: Waypoints },
          { label: "Facility Map", to: "/facility-map", icon: Radar },
          { label: "Pilot", to: "/pilot", icon: PlaneTakeoff },
        ],
      },
    ],
  },
  {
    id: "operations",
    label: "Operations",
    prefixes: ["/ops"],
    groups: [
      {
        items: [
          { label: "Airport", to: "/ops/airport", icon: Plane, permission: "tmu.program.read" },
          { label: "TMU", to: "/ops/tmu", icon: Gauge, anyOf: OPS_ANY },
          { label: "My Dashboard", to: "/ops/my", icon: LayoutDashboard, permission: "tmu.program.read" },
          { label: "FCA", to: "/ops/fca", icon: Waypoints, permission: "flow.fca.read" },
          { label: "IDST", to: "/ops/idst", icon: Timer, permission: "flow.fca.read" },
          { label: "Runway", to: "/ops/runway", icon: Split, permission: "flow.runway.read" },
          { label: "AADC", to: "/ops/aadc", icon: BarChart3, permission: "tmu.program.read" },
        ],
      },
    ],
  },
  {
    id: "admin",
    label: "Admin",
    prefixes: ["/admin"],
    groups: [
      {
        label: "Planning",
        prefix: "/admin/planning",
        items: [
          { label: "Events", to: "/admin/planning/events", icon: CalendarClock, permission: "events.plan.read" },
          { label: "Airport Configs", to: "/admin/planning/airport-configs", icon: Wind, permission: "events.plan.read" },
          { label: "Facility Documents", to: "/admin/planning/facility-documents", icon: FileText, permission: "facilities.docs.read" },
          { label: "Airport Surface", to: "/admin/planning/airport-surface", icon: MapPinned, permission: "events.plan.read" },
          { label: "Aircraft Profiles", to: "/admin/planning/aircraft-profiles", icon: Plane, permission: "flow.aircraft_profiles.read" },
        ],
      },
      {
        label: "Historical",
        prefix: "/admin/historical",
        items: [
          { label: "Overview", to: "/admin/historical", icon: TrendingUp, permission: "stats.data.read", exact: true },
          { label: "Dashboard", to: "/admin/historical/dashboard", icon: LayoutDashboard, permission: "stats.data.read" },
          { label: "Replay", to: "/admin/historical/replay", icon: Film, permission: "stats.data.read" },
          { label: "Delays", to: "/admin/historical/delays", icon: Timer, permission: "stats.data.read" },
          { label: "Taxi", to: "/admin/historical/taxi", icon: PlaneTakeoff, permission: "stats.data.read" },
        ],
      },
      {
        label: "Admin",
        items: [
          { label: "Access", to: "/admin/access", icon: ShieldCheck, permission: "access.users.read" },
          { label: "Audit", to: "/admin/audit", icon: ScrollText, permission: "audit.logs.read" },
          { label: "Jobs", to: "/admin/jobs", icon: Activity, permission: "system.jobs.read" },
          { label: "API Keys", to: "/admin/api-keys", icon: KeyRound, permission: "api_keys.key.read" },
          { label: "Discord", to: "/admin/discord", icon: MessageSquare, permission: "discord.config.read" },
        ],
      },
    ],
  },
];

/** The Admin area's landing — shown whenever any Admin-area link is. */
export const ADMIN_HOME: NavItem = { label: "Overview", to: "/admin", icon: LayoutDashboard, exact: true };

export function canSeeItem(me: Me | null | undefined, item: NavItem): boolean {
  if (item.permission) return hasPermission(me, item.permission);
  if (item.anyOf) return item.anyOf.some((p) => hasPermission(me, p));
  return true;
}

/** The area's groups with only the links `me` may use; empty groups dropped. */
export function visibleGroups(me: Me | null | undefined, area: NavArea): NavGroup[] {
  return area.groups
    .map((g) => ({ ...g, items: g.items.filter((i) => canSeeItem(me, i)) }))
    .filter((g) => g.items.length > 0);
}

export function canSeeArea(me: Me | null | undefined, area: NavArea): boolean {
  return visibleGroups(me, area).length > 0;
}

export function areaById(id: NavArea["id"]): NavArea {
  return AREAS.find((a) => a.id === id)!;
}

/** Whether `me` may open the Admin page: they can use at least one of its links. */
export function canSeeAdmin(me: Me | null | undefined): boolean {
  return canSeeArea(me, areaById("admin"));
}

const matchesPrefix = (pathname: string, prefix: string) =>
  pathname === prefix || pathname.startsWith(prefix.endsWith("/") ? prefix : `${prefix}/`);

export function areaForPath(pathname: string): NavArea | undefined {
  return AREAS.find((a) => a.prefixes.some((p) => matchesPrefix(pathname, p)));
}

/** The nav group whose `prefix` contains `pathname`, for pages below a group but not a nav item. */
export function groupForPath(pathname: string): NavGroup | undefined {
  return areaForPath(pathname)?.groups.find((g) => g.prefix && matchesPrefix(pathname, g.prefix));
}

/** The most specific nav item whose path contains `pathname` (for breadcrumbs). */
export function itemForPath(pathname: string): { area: NavArea; group: NavGroup; item: NavItem } | undefined {
  const area = areaForPath(pathname);
  if (!area) return undefined;
  let best: { area: NavArea; group: NavGroup; item: NavItem } | undefined;
  for (const group of area.groups) {
    for (const item of group.items) {
      const hit = item.exact ? pathname === item.to : matchesPrefix(pathname, item.to);
      if (hit && (!best || item.to.length > best.item.to.length)) best = { area, group, item };
    }
  }
  return best;
}

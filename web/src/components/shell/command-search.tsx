import {useEffect, useMemo, useState} from "react";
import {useNavigate} from "@tanstack/react-router";
import {CommandPalette, type CommandGroup, type CommandItem} from "@ois/ui";
import {
  CalendarClock,
  Home,
  KeyRound,
  LayoutDashboard,
  MapPinned,
  Megaphone,
  Plane,
  PlaneTakeoff,
  Radar,
  Settings as SettingsIcon,
  User as UserIcon,
  Wind,
} from "lucide-react";

import {useMe} from "@/lib/auth";
import {SCOPES, type ScopeId, parseScopePrefix} from "@/lib/command-scopes";
import {useDashboards} from "@/lib/dashboards";
import {useUpcomingEvents} from "@/lib/events";
import {useFacilityDirectory} from "@/lib/facilities";
import {useTraffic} from "@/lib/fca";
import {fuzzyMatch, rankAircraft} from "@/lib/fuzzy";
import {AREAS, type NavItem, canSeeItem, visibleGroups} from "@/lib/nav";
import {hasPermission} from "@/lib/permissions";
import {useTmis} from "@/lib/tmu";

/** Rows per group in the blended "All" view, and in a single focused scope. */
const LIMIT = 6;
const SCOPED_LIMIT = 20;

/** Global ⌘K / Ctrl+K search. Mount once; open with the hotkey or `openCommandSearch()`. */
export function CommandSearch() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener(OPEN_EVENT, onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener(OPEN_EVENT, onOpen);
    };
  }, []);
  // Only mount the sources (live traffic, events, TMIs, dashboards) while the palette is open.
  return open ? <Palette onClose={() => setOpen(false)} /> : null;
}

const OPEN_EVENT = "ois:command-search";
export const openCommandSearch = () => window.dispatchEvent(new Event(OPEN_EVENT));

function rank<T>(query: string, items: readonly T[], text: (t: T) => string, limit: number): T[] {
  if (!query.trim()) return items.slice(0, limit);
  return items
    .map((t) => ({ t, m: fuzzyMatch(query, text(t)) }))
    .filter((x): x is { t: T; m: NonNullable<typeof x.m> } => x.m != null)
    .sort((a, b) => b.m.score - a.m.score)
    .slice(0, limit)
    .map((x) => x.t);
}

const NAV_ITEMS = AREAS.flatMap((a) => a.groups.flatMap((g) => g.items));
const navItem = (to: string) => NAV_ITEMS.find((i) => i.to === to)!;

const PLACEHOLDER: Record<ScopeId, string> = {
  all: "Search pages, flights, airports, TMIs, events…",
  aircraft: "Search flights by callsign, route, or type…",
  tmis: "Search TMIs by facility or restriction…",
  events: "Search events…",
  dashboards: "Search your dashboards…",
  airports: "Search airports and facilities…",
  pages: "Search pages…",
};

function Palette({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const { data: me } = useMe();
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<ScopeId>("all");
  const q = query.trim();

  // A scope is offered only when its destination is reachable, and its source only fetches then.
  const canTmis = hasPermission(me, "tmu.tmi.read");
  const canEvents = hasPermission(me, "events.plan.read");
  const canDashboards = canSeeItem(me, navItem("/ops/my")) && hasPermission(me, "auth.profile.read");
  const scopes = SCOPES.filter(
    (s) =>
      (s.id !== "tmis" || canTmis) && (s.id !== "events" || canEvents) && (s.id !== "dashboards" || canDashboards),
  );

  const traffic = useTraffic();
  const facilities = useFacilityDirectory();
  const events = useUpcomingEvents({ enabled: canEvents });
  const tmis = useTmis(undefined, { enabled: canTmis });
  const dashboards = useDashboards({ enabled: canDashboards });

  const pages = useMemo(() => {
    const account: NavItem[] = me
      ? [
          { label: "Profile", to: "/profile", icon: UserIcon },
          { label: "Settings", to: "/settings", icon: SettingsIcon },
          ...(hasPermission(me, "api_keys.key.create") ? [{ label: "API keys", to: "/api-keys", icon: KeyRound }] : []),
        ]
      : [];
    const areaPages = AREAS.flatMap((area) =>
      visibleGroups(me, area).flatMap((g) =>
        g.items.map((i) => ({ ...i, context: [area.label, g.label].filter(Boolean).join(" · ") })),
      ),
    );
    return [{ label: "Home", to: "/", icon: Home, context: "" }, ...areaPages, ...account.map((i) => ({ ...i, context: "Account" }))];
  }, [me]);

  // `@tmi ` / `@airc ` jump straight to a scope and drop the prefix from the query.
  const onQueryChange = (next: string) => {
    const typed = parseScopePrefix(next, scopes);
    if (typed) {
      setScope(typed.scope as ScopeId);
      setQuery(typed.rest);
    } else {
      setQuery(next);
    }
  };

  const limit = scope === "all" ? LIMIT : SCOPED_LIMIT;

  const pageItems = (): CommandItem[] =>
    rank(q, pages, (p) => `${p.label} ${p.context}`, limit).map((p) => ({
      id: `page:${p.to}`,
      label: p.label,
      sublabel: p.context,
      icon: p.icon,
      onSelect: () => void navigate({ to: p.to }),
    }));

  // An ICAO-shaped query offers that airport's page — plus its planning pages inside the Airport data
  // scope, so the blended view stays uncluttered — each gated like its nav link, then facility maps.
  const airportItems = (): CommandItem[] => {
    const icao = q.toUpperCase().replace(/[^A-Z0-9]/g, "");
    const icaoPages: CommandItem[] = !/^[A-Z0-9]{3,4}$/.test(icao)
      ? []
      : [
          { item: navItem("/ops/airport"), label: `${icao} airport`, sublabel: "Operations · Airport", icon: Plane, search: { icao } },
          { item: navItem("/admin/planning/airport-configs"), label: `${icao} airport configs`, sublabel: "Planning", icon: Wind },
          { item: navItem("/admin/planning/airport-surface"), label: `${icao} airport surface`, sublabel: "Planning", icon: MapPinned },
        ]
          .filter((p, i) => (i === 0 || scope === "airports") && canSeeItem(me, p.item))
          .map((p) => ({
            id: `airport:${p.item.to}:${icao}`,
            label: p.label,
            sublabel: p.sublabel,
            icon: p.icon,
            onSelect: () => void navigate({ to: p.item.to, search: p.search }),
          }));
    const facilityItems = rank(q, facilities.data ?? [], (f) => `${f.id} ${f.name ?? ""}`, limit).map((f) => ({
      id: `facility:${f.id}`,
      label: f.name ? `${f.id} · ${f.name}` : f.id,
      sublabel: `${f.kind.toUpperCase()} · facility map`,
      icon: Radar,
      onSelect: () => void navigate({ to: "/facility-map/$facilityId", params: { facilityId: f.id } }),
    }));
    return [...icaoPages, ...facilityItems];
  };

  const flightItems = (): CommandItem[] =>
    rankAircraft(q, traffic.data ?? [], limit).map(({ ac }) => ({
      id: `flight:${ac.callsign}`,
      label: ac.callsign,
      sublabel: [ac.dep, ac.arr].filter(Boolean).join(" → "),
      icon: PlaneTakeoff,
      onSelect: () => void navigate({ to: "/advisories/fcas", search: { flight: ac.callsign } }),
    }));

  const eventItems = (): CommandItem[] =>
    rank(q, events.data ?? [], (e) => `${e.title} ${e.facility}`, limit).map((e) => ({
      id: `event:${e.id}`,
      label: e.title,
      sublabel: e.facility,
      icon: CalendarClock,
      onSelect: () => void navigate({ to: "/admin/planning/events/$eventId", params: { eventId: String(e.id) } }),
    }));

  // TMIs have no page of their own: every row opens the TMU restrictions tab.
  const tmiItems = (): CommandItem[] =>
    rank(q, tmis.data ?? [], (t) => `${t.requesting} ${t.providing} ${t.decoded ?? t.restriction} ${t.status}`, limit).map(
      (t) => ({
        id: `tmi:${t.id}`,
        label: t.decoded ?? t.restriction,
        sublabel: `${t.requesting}→${t.providing} · ${t.status}`,
        icon: Megaphone,
        onSelect: () => void navigate({ to: "/ops/tmu", search: { tab: "restrictions" } }),
      }),
    );

  const dashboardItems = (): CommandItem[] =>
    rank(q, dashboards.data?.dashboards ?? [], (d) => d.name, limit).map((d) => ({
      id: `dashboard:${d.id}`,
      label: d.name,
      sublabel: "Dashboard",
      icon: LayoutDashboard,
      onSelect: () => void navigate({ to: "/ops/my/$boardId", params: { boardId: d.id } }),
    }));

  const scopeLabel = SCOPES.find((s) => s.id === scope)!.label;
  const groups: CommandGroup[] = [];
  let empty = "No results.";
  if (scope === "all") {
    groups.push({ label: "Pages", items: pageItems() });
    if (q.length >= 2) {
      groups.push({ label: "Airports & facilities", items: airportItems() });
      groups.push({ label: "Flights", items: flightItems() });
      if (canTmis) groups.push({ label: "TMIs", items: tmiItems() });
      if (canEvents) groups.push({ label: "Events", items: eventItems() });
      if (canDashboards) groups.push({ label: "Dashboards", items: dashboardItems() });
    }
  } else {
    const source = {
      aircraft: { items: flightItems, loading: traffic.isLoading },
      tmis: { items: tmiItems, loading: tmis.isLoading },
      events: { items: eventItems, loading: events.isLoading },
      dashboards: { items: dashboardItems, loading: dashboards.isLoading },
      airports: { items: airportItems, loading: facilities.isLoading },
      pages: { items: pageItems, loading: false },
    }[scope];
    groups.push({ label: scopeLabel, items: source.items() });
    empty = source.loading
      ? `Loading ${scopeLabel.toLowerCase()}…`
      : scope === "aircraft" && !q
        ? "Type a callsign, route, or aircraft type."
        : `No ${scopeLabel.toLowerCase()} match.`;
  }

  return (
    <CommandPalette
      open
      onClose={onClose}
      query={query}
      onQueryChange={onQueryChange}
      groups={groups}
      scopes={scopes}
      scope={scope}
      onScopeChange={(s) => setScope(s as ScopeId)}
      placeholder={PLACEHOLDER[scope]}
      empty={empty}
      footer={
        <span>
          <kbd className="font-mono">↑↓</kbd> move · <kbd className="font-mono">↵</kbd> open ·{" "}
          <kbd className="font-mono">tab</kbd> scope · <kbd className="font-mono">esc</kbd> close
        </span>
      }
    />
  );
}

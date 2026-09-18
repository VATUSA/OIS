import {useEffect, useMemo, useState} from "react";
import {useNavigate, useRouterState} from "@tanstack/react-router";
import {CommandPalette, type CommandGroup, type CommandItem, useToast} from "@ois/ui";
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
  Star,
  Settings as SettingsIcon,
  User as UserIcon,
  Wind,
} from "lucide-react";

import {useMe} from "@/lib/auth";
import {SCOPES, type ScopeId, icaoRows, parseScopePrefix, tmiRow} from "@/lib/command-scopes";
import {useDashboards} from "@/lib/dashboards";
import {useUpcomingEvents} from "@/lib/events";
import {
  type Favorite,
  type FavoriteKind,
  type FavoriteScopes,
  type FavoriteSources,
  canSeeFavorite,
  favoriteHref,
  favoriteUnavailable,
  useFavorites,
  withPinnedFavorites,
} from "@/lib/favorites";
import {useFacilityDirectory} from "@/lib/facilities";
import {useTraffic} from "@/lib/fca";
import {fuzzyMatch, rankAircraft} from "@/lib/fuzzy";
import {AREAS, type NavItem, canSeeItem, visibleGroups} from "@/lib/nav";
import {hasPermission} from "@/lib/permissions";
import {useTmis} from "@/lib/tmu";

import {usePageTitle} from "./page-meta";

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
  const { data: me } = useMe();
  // Only mount the sources (live traffic, events, TMIs, dashboards) while the palette is open. While
  // it's closed, ⌘⇧F favorites the current page instead (signed-in only: favorites are per user).
  if (open) return <Palette onClose={() => setOpen(false)} />;
  return me ? <FavoriteCurrentPage /> : null;
}

/** ⌘⇧F / Ctrl+Shift+F with the palette closed: toggle the current page as a favorite. */
function FavoriteCurrentPage() {
  const favorites = useFavorites();
  const toast = useToast();
  const location = useRouterState({ select: (s) => s.location });
  // The page's own title, not the nav link's: `itemForPath` is a prefix match, so it would label
  // every event "Events" and every board "My Dashboard" — favorites that can't be told apart.
  const title = usePageTitle();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "f")) return;
      e.preventDefault();
      const label = title ?? location.pathname;
      // Keyed on the full href, so two airports' config pages are two favorites, not one.
      const added = favorites.toggle({ kind: "page", id: location.href, label, href: location.href });
      if (added != null) toast.success(added ? `Added ${label} to favorites` : `Removed ${label} from favorites`);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [favorites, toast, location, title]);
  return null;
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

/** The icon for each ICAO row, keyed by its destination (the rows themselves live in `lib`). */
const ICAO_ROW_ICON: Record<string, typeof Plane> = {
  "/ops/airport": Plane,
  "/admin/planning/airport-configs": Wind,
  "/admin/planning/airport-surface": MapPinned,
};

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

  const favorites = useFavorites();
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

  // Favorites are per user and live in the signed-in preferences API. The shell (and so the palette)
  // renders on every public path but `/`, so without this a signed-out visitor gets a star that
  // 401s and toasts "Couldn't save favorites" — `FavoriteCurrentPage` is gated the same way.
  const favoritesEnabled = me != null;
  const star = (item: CommandItem, fav: Favorite): CommandItem =>
    favoritesEnabled
      ? {
          ...item,
          starred: favorites.isFavorite(fav.kind, fav.id),
          onToggleStar: () => favorites.toggle(fav),
        }
      : item;

  const pageItems = (): CommandItem[] =>
    rank(q, pages, (p) => `${p.label} ${p.context}`, limit).map((p) =>
      star(
        { id: `page:${p.to}`, label: p.label, sublabel: p.context, icon: p.icon, onSelect: () => void navigate({ to: p.to }) },
        { kind: "page", id: p.to, label: p.label, href: p.to },
      ),
    );

  // An ICAO-shaped query offers that airport's page — plus its planning pages inside the Airport data
  // scope, so the blended view stays uncluttered — each gated like its nav link, then facility maps.
  const airportItems = (): CommandItem[] => {
    const icao = q.toUpperCase().replace(/[^A-Z0-9]/g, "");
    const icaoPages: CommandItem[] = icaoRows(icao)
      .filter((p, i) => (i === 0 || scope === "airports") && canSeeItem(me, navItem(p.to)))
      .map((p) =>
        star(
          {
            id: `airport:${p.to}:${icao}`,
            label: p.label,
            sublabel: p.sublabel,
            icon: ICAO_ROW_ICON[p.to],
            // Each row names an airport, so each row opens that airport — not the page's empty picker.
            onSelect: () => void navigate({ to: p.to, search: p.search }),
          },
          // Every ICAO row carries its airport, so the favorite reopens the same airport too —
          // built from the row's own `search` so it can't drift from where the row lands.
          { kind: "airport", id: `${p.to}:${icao}`, label: p.label, href: favoriteHref(p) },
        ),
      );
    const facilityItems = rank(q, facilities.data ?? [], (f) => `${f.id} ${f.name ?? ""}`, limit).map((f) => {
      const label = f.name ? `${f.id} · ${f.name}` : f.id;
      return star(
        {
          id: `facility:${f.id}`,
          label,
          sublabel: `${f.kind.toUpperCase()} · facility map`,
          icon: Radar,
          onSelect: () => void navigate({ to: "/facility-map/$facilityId", params: { facilityId: f.id } }),
        },
        { kind: "airport", id: `facility:${f.id}`, label, href: `/facility-map/${f.id}` },
      );
    });
    return [...icaoPages, ...facilityItems];
  };

  const flightItems = (): CommandItem[] =>
    rankAircraft(q, traffic.data ?? [], limit).map(({ ac }) =>
      star(
        {
          id: `flight:${ac.callsign}`,
          label: ac.callsign,
          sublabel: [ac.dep, ac.arr].filter(Boolean).join(" → "),
          icon: PlaneTakeoff,
          onSelect: () => void navigate({ to: "/advisories/fcas", search: { flight: ac.callsign } }),
        },
        {
          kind: "aircraft",
          id: ac.callsign,
          label: ac.callsign,
          href: `/advisories/fcas?flight=${encodeURIComponent(ac.callsign)}`,
        },
      ),
    );

  const eventItems = (): CommandItem[] =>
    rank(q, events.data ?? [], (e) => `${e.title} ${e.facility}`, limit).map((e) =>
      star(
        {
          id: `event:${e.id}`,
          label: e.title,
          sublabel: e.facility,
          icon: CalendarClock,
          onSelect: () => void navigate({ to: "/admin/planning/events/$eventId", params: { eventId: String(e.id) } }),
        },
        { kind: "event", id: String(e.id), label: e.title, href: `/admin/planning/events/${e.id}` },
      ),
    );

  // TMIs have no page of their own: every row opens the TMU restrictions tab, filtered to the TMI's
  // facility — otherwise every row of a 20-row list lands on the same unfiltered page.
  const tmiItems = (): CommandItem[] =>
    rank(q, tmis.data ?? [], (t) => `${t.requesting} ${t.providing} ${t.decoded ?? t.restriction} ${t.status}`, limit).map(
      (t) =>
        star(
          {
            id: `tmi:${t.id}`,
            label: t.decoded ?? t.restriction,
            sublabel: `${t.requesting}→${t.providing} · ${t.status}`,
            icon: Megaphone,
            onSelect: () => void navigate(tmiRow(t)),
          },
          // The favorite reopens the same filtered tab the row lands on.
          { kind: "tmi", id: t.id, label: t.decoded ?? t.restriction, href: favoriteHref(tmiRow(t)) },
        ),
    );

  const dashboardItems = (): CommandItem[] =>
    rank(q, dashboards.data?.dashboards ?? [], (d) => d.name, limit).map((d) =>
      star(
        {
          id: `dashboard:${d.id}`,
          label: d.name,
          sublabel: "Dashboard",
          icon: LayoutDashboard,
          onSelect: () => void navigate({ to: "/ops/my/$boardId", params: { boardId: d.id } }),
        },
        { kind: "dashboard", id: d.id, label: d.name, href: `/ops/my/${d.id}` },
      ),
    );

  // A favorite is listed only while its kind and destination are still permitted, and marked gone
  // once its source has loaded without it — the row stays either way, so it can still be unstarred.
  const favoriteScopes: FavoriteScopes = { tmis: canTmis, events: canEvents, dashboards: canDashboards };
  const sources: FavoriteSources = {
    aircraft: traffic.data,
    tmis: tmis.data,
    events: events.data,
    dashboards: dashboards.data?.dashboards,
  };

  const FAVORITE_KIND: Record<ScopeId, FavoriteKind | null> = {
    all: null,
    aircraft: "aircraft",
    tmis: "tmi",
    events: "event",
    dashboards: "dashboard",
    airports: "airport",
    pages: "page",
  };
  const favoriteItems = (): CommandItem[] => {
    const kind = FAVORITE_KIND[scope];
    const visible = favorites.items.filter((f) => (kind == null || f.kind === kind) && canSeeFavorite(me, f, favoriteScopes));
    return rank(q, visible, (f) => f.label, visible.length).map((f) =>
      star(
        {
          id: `favorite:${f.kind}:${f.id}`,
          label: f.label,
          sublabel: favoriteUnavailable(f, sources) ? "Unavailable" : f.kind,
          icon: Star,
          onSelect: () => void navigate({ href: f.href }),
        },
        f,
      ),
    );
  };

  const { label: scopeLabel, noun: scopeNoun } = SCOPES.find((s) => s.id === scope)!;
  const scoped: CommandGroup[] = [];
  let empty = "No results.";
  if (scope === "all") {
    scoped.push({ label: "Pages", items: pageItems() });
    if (q.length >= 2) {
      scoped.push({ label: "Airports & facilities", items: airportItems() });
      scoped.push({ label: "Flights", items: flightItems() });
      if (canTmis) scoped.push({ label: "TMIs", items: tmiItems() });
      if (canEvents) scoped.push({ label: "Events", items: eventItems() });
      if (canDashboards) scoped.push({ label: "Dashboards", items: dashboardItems() });
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
    scoped.push({ label: scopeLabel, items: source.items() });
    empty = source.loading
      ? `Loading ${scopeNoun}…`
      : scope === "aircraft" && !q
        ? "Type a callsign, route, or aircraft type."
        : `No ${scopeNoun} match.`;
  }
  const groups = withPinnedFavorites(favoritesEnabled, { label: "Favorites", items: favoriteItems() }, scoped);

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

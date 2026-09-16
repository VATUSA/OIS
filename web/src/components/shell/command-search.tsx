import {useEffect, useMemo, useState} from "react";
import {useNavigate} from "@tanstack/react-router";
import {CommandPalette, type CommandGroup} from "@ois/ui";
import {CalendarClock, Home, KeyRound, Plane, PlaneTakeoff, Radar, Settings as SettingsIcon, User as UserIcon} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useUpcomingEvents} from "@/lib/events";
import {useFacilityDirectory} from "@/lib/facilities";
import {useTraffic} from "@/lib/fca";
import {fuzzyMatch, rankAircraft} from "@/lib/fuzzy";
import {AREAS, type NavItem, canSeeItem, visibleGroups} from "@/lib/nav";
import {hasPermission} from "@/lib/permissions";

const LIMIT = 6;

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
  // Only mount the sources (live traffic, events) while the palette is open.
  return open ? <Palette onClose={() => setOpen(false)} /> : null;
}

const OPEN_EVENT = "ois:command-search";
export const openCommandSearch = () => window.dispatchEvent(new Event(OPEN_EVENT));

function rank<T>(query: string, items: readonly T[], text: (t: T) => string): T[] {
  if (!query.trim()) return items.slice(0, LIMIT);
  return items
    .map((t) => ({ t, m: fuzzyMatch(query, text(t)) }))
    .filter((x): x is { t: T; m: NonNullable<typeof x.m> } => x.m != null)
    .sort((a, b) => b.m.score - a.m.score)
    .slice(0, LIMIT)
    .map((x) => x.t);
}

function Palette({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const { data: me } = useMe();
  const [query, setQuery] = useState("");
  const q = query.trim();

  const canEvents = hasPermission(me, "events.plan.read");
  const traffic = useTraffic();
  const facilities = useFacilityDirectory();
  const events = useUpcomingEvents({ enabled: canEvents });
  const airportItem = AREAS.flatMap((a) => a.groups.flatMap((g) => g.items)).find((i) => i.to === "/ops/airport")!;

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

  const groups: CommandGroup[] = [
    {
      label: "Pages",
      items: rank(q, pages, (p) => `${p.label} ${p.context}`).map((p) => ({
        id: `page:${p.to}`,
        label: p.label,
        sublabel: p.context,
        icon: p.icon,
        onSelect: () => void navigate({ to: p.to }),
      })),
    },
  ];

  if (q.length >= 2) {
    const icao = q.toUpperCase().replace(/[^A-Z0-9]/g, "");
    const airportItems =
      /^[A-Z0-9]{3,4}$/.test(icao) && canSeeItem(me, airportItem)
        ? [
            {
              id: `airport:${icao}`,
              label: `${icao} airport`,
              sublabel: "Operations · Airport",
              icon: Plane,
              onSelect: () => void navigate({ to: "/ops/airport", search: { icao } }),
            },
          ]
        : [];
    const facilityItems = rank(q, facilities.data ?? [], (f) => `${f.id} ${f.name ?? ""}`).map((f) => ({
      id: `facility:${f.id}`,
      label: f.name ? `${f.id} · ${f.name}` : f.id,
      sublabel: `${f.kind.toUpperCase()} · facility map`,
      icon: Radar,
      onSelect: () => void navigate({ to: "/facility-map/$facilityId", params: { facilityId: f.id } }),
    }));
    groups.push({ label: "Airports & facilities", items: [...airportItems, ...facilityItems] });

    const flights = rankAircraft(q, traffic.data ?? [], LIMIT);
    groups.push({
      label: "Flights",
      items: flights.map(({ ac }) => ({
        id: `flight:${ac.callsign}`,
        label: ac.callsign,
        sublabel: [ac.dep, ac.arr].filter(Boolean).join(" → "),
        icon: PlaneTakeoff,
        onSelect: () => void navigate({ to: "/advisories/fcas", search: { flight: ac.callsign } }),
      })),
    });

    if (canEvents) {
      groups.push({
        label: "Events",
        items: rank(q, events.data ?? [], (e) => `${e.title} ${e.facility}`).map((e) => ({
          id: `event:${e.id}`,
          label: e.title,
          sublabel: e.facility,
          icon: CalendarClock,
          onSelect: () =>
            void navigate({ to: "/admin/planning/events/$eventId", params: { eventId: String(e.id) } }),
        })),
      });
    }
  }

  return (
    <CommandPalette
      open
      onClose={onClose}
      query={query}
      onQueryChange={setQuery}
      groups={groups}
      placeholder="Search pages, flights, airports, events…"
      footer={
        <span>
          <kbd className="font-mono">↑↓</kbd> move · <kbd className="font-mono">↵</kbd> open ·{" "}
          <kbd className="font-mono">esc</kbd> close
        </span>
      }
    />
  );
}

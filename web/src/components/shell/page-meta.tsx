import {createContext, type ReactNode, useCallback, useContext, useLayoutEffect, useRef, useState} from "react";
import {useRouterState} from "@tanstack/react-router";
import type {LucideIcon} from "lucide-react";

import {itemForPath} from "@/lib/nav";

export type ViewOption = { value: string; label: string; icon?: LucideIcon };

/** What a route declares about itself in `staticData` (typed in router.tsx). */
export type RouteMeta = {
  layout?: "full" | "wide";
  title?: string;
  subtitle?: string;
  icon?: LucideIcon;
  views?: readonly ViewOption[];
};

/** Values a page sets at runtime (an event's name, a list count, header actions). */
export type PageHeaderOverride = {
  title?: string;
  subtitle?: ReactNode;
  count?: number | null;
  actions?: ReactNode;
  /** `null` hides the route's view switch on this page state (e.g. a tab it doesn't apply to). */
  views?: null;
};

const EMPTY: PageHeaderOverride = {};

type Ctx = { override: PageHeaderOverride; set: (o: PageHeaderOverride) => void };
const PageMetaContext = createContext<Ctx | null>(null);

export function PageMetaProvider({ children }: { children: ReactNode }) {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const pathRef = useRef(path);
  pathRef.current = path;
  // Overrides are stamped with the path they were set on, so a new page starts from its route meta
  // without a reset effect (which would run after — and wipe — the new page's own mount-time set).
  const [stamped, setStamped] = useState<{ path: string; override: PageHeaderOverride }>({ path, override: {} });
  const set = useCallback((override: PageHeaderOverride) => setStamped({ path: pathRef.current, override }), []);
  const override = stamped.path === path ? stamped.override : EMPTY;
  return <PageMetaContext.Provider value={{ override, set }}>{children}</PageMetaContext.Provider>;
}

export function usePageHeaderOverride(): PageHeaderOverride {
  return useContext(PageMetaContext)?.override ?? {};
}

/**
 * Set this page's dynamic header values. Call with a memo-stable object (or primitives) — it re-applies
 * whenever the fields change.
 */
export function usePageHeader(o: PageHeaderOverride) {
  const ctx = useContext(PageMetaContext);
  const set = ctx?.set;
  useLayoutEffect(() => {
    set?.(o);
  }, [set, o.title, o.subtitle, o.count, o.actions, o.views]);
}

/** The deepest matched route's value for each meta key (a parent can set it for its children). */
export function useRouteMeta(): RouteMeta {
  return useRouterState({
    select: (s) => {
      const meta: RouteMeta = {};
      for (let i = s.matches.length - 1; i >= 0; i--) {
        const d = (s.matches[i].staticData ?? {}) as RouteMeta;
        meta.layout ??= d.layout;
        meta.title ??= d.title;
        meta.subtitle ??= d.subtitle;
        meta.icon ??= d.icon;
        meta.views ??= d.views;
      }
      return meta;
    },
  });
}

/**
 * The title this page is showing — a page override, else its route's, else the nav link's. The same
 * expression `Frame` renders, so anything naming a page from outside the header (favoriting it, the
 * recents list) cannot drift from what the user is looking at. `itemForPath` alone is a *prefix*
 * match built for breadcrumbs, so on its own it would call every event "Events" (VATUSA/OIS#312).
 */
export function usePageTitle(): string | undefined {
  const meta = useRouteMeta();
  const override = usePageHeaderOverride();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  return override.title ?? meta.title ?? itemForPath(pathname)?.item.label;
}

/** The page's current view (`?view=`), defaulting to its first declared view. */
export function useView(): string | undefined {
  const meta = useRouteMeta();
  const view = useRouterState({
    select: (s) => (s.location.search as Record<string, unknown> | undefined)?.view,
  });
  const values = meta.views?.map((v) => v.value) ?? [];
  return typeof view === "string" && values.includes(view) ? view : values[0];
}

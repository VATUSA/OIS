import {createContext, type ReactNode, useContext, useLayoutEffect, useState} from "react";
import {useRouterState} from "@tanstack/react-router";
import type {LucideIcon} from "lucide-react";

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
};

type Ctx = { override: PageHeaderOverride; set: (o: PageHeaderOverride) => void };
const PageMetaContext = createContext<Ctx | null>(null);

export function PageMetaProvider({ children }: { children: ReactNode }) {
  const [override, set] = useState<PageHeaderOverride>({});
  const path = useRouterState({ select: (s) => s.location.pathname });
  // A new page starts from its route meta, never the previous page's overrides.
  useLayoutEffect(() => set({}), [path]);
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
  }, [set, o.title, o.subtitle, o.count, o.actions]);
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

/** The page's current view (`?view=`), defaulting to its first declared view. */
export function useView(): string | undefined {
  const meta = useRouteMeta();
  const view = useRouterState({
    select: (s) => (s.location.search as Record<string, unknown> | undefined)?.view,
  });
  const values = meta.views?.map((v) => v.value) ?? [];
  return typeof view === "string" && values.includes(view) ? view : values[0];
}

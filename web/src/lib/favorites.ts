import {useToast} from "@ois/ui";
import {useQueryClient} from "@tanstack/react-query";

import type {Me} from "./auth";
import {canOpenPath, canSeeItem, itemForPath} from "./nav";
import {hasPermission} from "./permissions";
import {usePreferences, useSavePreferences} from "./preferences";

// ⌘⇧F is matched in one place, next to the palette that also fires it — re-exported here so this
// module stays the one import for everything favorites (VATUSA/OIS#312).
export { isFavoriteHotkey } from "@ois/ui";

const NAMESPACE = "favorites";

export type FavoriteKind = "aircraft" | "tmi" | "event" | "dashboard" | "airport" | "page";

/** A favorited command-search entity. `href` is where it opens; `label` is shown even when the entity is gone. */
export type Favorite = { kind: FavoriteKind; id: string; label: string; href: string };

/** The client-owned shape stored in the `favorites` preferences namespace. */
type FavoritesPrefs = { items: Favorite[] };

export const favoriteKey = (f: Pick<Favorite, "kind" | "id">) => `${f.kind}:${f.id}`;

/**
 * A favorite's `href` for a command-search row destination. Built from the row's own `to`/`search`
 * so the favorite always reopens exactly what the row opens, even when that builder's params change.
 */
export const favoriteHref = ({ to, search }: { to: string; search: Record<string, string> }) =>
  `${to}?${new URLSearchParams(search)}`;

/** Search params that change how a page is shown, not what it shows (`Header`'s view switch). */
const PRESENTATIONAL_PARAMS = ["view"];

/**
 * The key and href for favoriting the page at `href`: the subject, minus presentational params. Keying
 * on the raw href made `/ops/tmu?view=board` and `?view=table` two identical "TMU" favorites, neither
 * of which the palette's own `/ops/tmu` row recognised as starred (VATUSA/OIS#339). An href with none
 * of those params is returned untouched, so favorites already stored keep their keys.
 */
export function pageFavoriteHref(href: string): string {
  const url = new URL(href, "http://x");
  if (!PRESENTATIONAL_PARAMS.some((p) => url.searchParams.has(p))) return href;
  for (const p of PRESENTATIONAL_PARAMS) url.searchParams.delete(p);
  return `${url.pathname}${url.search}${url.hash}`;
}

/** The permission a favorite's kind needs before it is worth listing; the rest are gated by destination. */
const KIND_PERMISSION: Partial<Record<FavoriteKind, string>> = {
  tmi: "tmu.tmi.read",
  event: "events.plan.read",
  dashboard: "auth.profile.read",
};

/**
 * Whether `me` may still see a stored favorite: its kind's permission, then its destination gated
 * exactly like the nav link that reaches it. A favorite the user has lost access to stays in storage
 * but drops out of the list.
 */
export function canSeeFavorite(me: Me | null | undefined, f: Favorite): boolean {
  const permission = KIND_PERMISSION[f.kind];
  if (permission && !hasPermission(me, permission)) return false;
  const path = f.href.split("?")[0];
  if (path.startsWith("/admin")) return canOpenPath(me, path);
  const hit = itemForPath(path);
  return hit ? canSeeItem(me, hit.item) : true;
}

/** The live entities a favorite can be checked against; `undefined` means that source hasn't loaded. */
export type FavoriteSources = {
  aircraft?: readonly { callsign: string }[];
  tmis?: readonly { id: string }[];
  events?: readonly { id: number | string }[];
  dashboards?: readonly { id: string }[];
};

/**
 * Whether a favorite's entity is gone — its source has loaded and doesn't hold it. Pages and airports
 * have no source to go missing from. The row stays listed either way, so it can still be unstarred.
 */
export function unavailable(f: Favorite, sources: FavoriteSources): boolean {
  const missing = <T,>(data: readonly T[] | undefined, id: (t: T) => string) =>
    data != null && !data.some((t) => id(t) === f.id);
  switch (f.kind) {
    case "aircraft":
      return missing(sources.aircraft, (a) => a.callsign);
    case "tmi":
      return missing(sources.tmis, (t) => t.id);
    case "event":
      return missing(sources.events, (e) => String(e.id));
    case "dashboard":
      return missing(sources.dashboards, (d) => d.id);
    default:
      return false;
  }
}

export function isFavorite(items: readonly Favorite[], kind: FavoriteKind, id: string): boolean {
  return items.some((f) => f.kind === kind && f.id === id);
}

/** Remove `fav` when it's already a favorite (matched by kind + id), else add it to the front. */
export function toggleFavorite(items: readonly Favorite[], fav: Favorite): Favorite[] {
  const key = favoriteKey(fav);
  return items.some((f) => favoriteKey(f) === key) ? items.filter((f) => favoriteKey(f) !== key) : [fav, ...items];
}

/**
 * This user's favorites, with an optimistic toggle persisted to their preferences. `toggle` returns
 * whether the entity is now a favorite, or `null` (and changes nothing) while the stored list is
 * still loading — saving before then would overwrite it. Pass `enabled: false` when signed out:
 * favorites are per user, so there is nothing to fetch and nothing that could be saved.
 */
export function useFavorites(enabled = true) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const prefs = usePreferences<FavoritesPrefs>(NAMESPACE, { enabled });
  const save = useSavePreferences<FavoritesPrefs>(NAMESPACE);
  const items = prefs.data?.items ?? [];

  const toggle = (fav: Favorite): boolean | null => {
    if (!prefs.isSuccess) return null;
    const previous = queryClient.getQueryData<FavoritesPrefs | null>(["preferences", NAMESPACE]) ?? null;
    const next = { items: toggleFavorite(previous?.items ?? [], fav) };
    queryClient.setQueryData(["preferences", NAMESPACE], next);
    save.mutate(next, {
      onError: () => {
        queryClient.setQueryData(["preferences", NAMESPACE], previous);
        toast.error("Couldn't save favorites");
      },
    });
    return isFavorite(next.items, fav.kind, fav.id);
  };

  return { items, isFavorite: (kind: FavoriteKind, id: string) => isFavorite(items, kind, id), toggle };
}

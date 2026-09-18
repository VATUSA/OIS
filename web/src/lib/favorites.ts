import {useToast} from "@ois/ui";
import {useQueryClient} from "@tanstack/react-query";

import type {Me} from "./auth";
import {canOpenPath, canSeeItem, itemForPath} from "./nav";
import {usePreferences, useSavePreferences} from "./preferences";

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

export function isFavorite(items: readonly Favorite[], kind: FavoriteKind, id: string): boolean {
  return items.some((f) => f.kind === kind && f.id === id);
}

/** Remove `fav` when it's already a favorite (matched by kind + id), else add it to the front. */
export function toggleFavorite(items: readonly Favorite[], fav: Favorite): Favorite[] {
  const key = favoriteKey(fav);
  return items.some((f) => favoriteKey(f) === key) ? items.filter((f) => favoriteKey(f) !== key) : [fav, ...items];
}

/**
 * The palette's groups with Favorites pinned first — the whole point of the feature is that your
 * most-used things sit above every scoped result. A signed-out viewer gets no Favorites group at
 * all: favorites are per user and live behind the signed-in preferences API.
 */
export function withPinnedFavorites<G>(signedIn: boolean, favorites: G, rest: readonly G[]): G[] {
  return signedIn ? [favorites, ...rest] : [...rest];
}

/** Which favorite kinds the viewer may read at all — the scope-level gate the palette already computes. */
export type FavoriteScopes = { tmis: boolean; events: boolean; dashboards: boolean };

/**
 * Whether a stored favorite may still be listed: its kind's source has to be readable, and its
 * destination has to be a page the viewer can open. Favorites outlive a permission change, so this
 * is re-checked on every render rather than trusted from write time.
 */
export function canSeeFavorite(me: Me | null | undefined, f: Favorite, scopes: FavoriteScopes): boolean {
  if (
    (f.kind === "tmi" && !scopes.tmis) ||
    (f.kind === "event" && !scopes.events) ||
    (f.kind === "dashboard" && !scopes.dashboards)
  ) {
    return false;
  }
  const path = f.href.split("?")[0];
  if (path.startsWith("/admin")) return canOpenPath(me, path);
  const hit = itemForPath(path);
  return hit ? canSeeItem(me, hit.item) : true;
}

/** The loaded rows each favorite kind is matched against; `undefined` means "not loaded yet". */
export type FavoriteSources = {
  aircraft?: readonly { callsign: string }[];
  tmis?: readonly { id: string }[];
  events?: readonly { id: number }[];
  dashboards?: readonly { id: string }[];
};

/**
 * Whether a favorite's entity is gone — its source has loaded without it. An unloaded source is
 * never "gone", so a favorite doesn't flash "Unavailable" while the palette's queries settle.
 */
export function favoriteUnavailable(f: Favorite, sources: FavoriteSources): boolean {
  const missing = <T,>(rows: readonly T[] | undefined, id: (t: T) => string) =>
    rows != null && !rows.some((t) => id(t) === f.id);
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

/**
 * This user's favorites, with an optimistic toggle persisted to their preferences. `toggle` returns
 * whether the entity is now a favorite, or `null` (and changes nothing) while the stored list is
 * still loading — saving before then would overwrite it.
 */
export function useFavorites() {
  const queryClient = useQueryClient();
  const toast = useToast();
  const prefs = usePreferences<FavoritesPrefs>(NAMESPACE);
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

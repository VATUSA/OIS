import {useToast} from "@ois/ui";
import {useQueryClient} from "@tanstack/react-query";

import {usePreferences, useSavePreferences} from "./preferences";

const NAMESPACE = "favorites";

export type FavoriteKind = "aircraft" | "tmi" | "event" | "dashboard" | "airport" | "page";

/** A favorited command-search entity. `href` is where it opens; `label` is shown even when the entity is gone. */
export type Favorite = { kind: FavoriteKind; id: string; label: string; href: string };

/** The client-owned shape stored in the `favorites` preferences namespace. */
type FavoritesPrefs = { items: Favorite[] };

export const favoriteKey = (f: Pick<Favorite, "kind" | "id">) => `${f.kind}:${f.id}`;

export function isFavorite(items: readonly Favorite[], kind: FavoriteKind, id: string): boolean {
  return items.some((f) => f.kind === kind && f.id === id);
}

/** Remove `fav` when it's already a favorite (matched by kind + id), else add it to the front. */
export function toggleFavorite(items: readonly Favorite[], fav: Favorite): Favorite[] {
  const key = favoriteKey(fav);
  return items.some((f) => favoriteKey(f) === key) ? items.filter((f) => favoriteKey(f) !== key) : [fav, ...items];
}

/** This user's favorites, with an optimistic toggle persisted to their preferences. */
export function useFavorites() {
  const queryClient = useQueryClient();
  const toast = useToast();
  const prefs = usePreferences<FavoritesPrefs>(NAMESPACE);
  const save = useSavePreferences<FavoritesPrefs>(NAMESPACE);
  const items = prefs.data?.items ?? [];

  const toggle = (fav: Favorite): boolean => {
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

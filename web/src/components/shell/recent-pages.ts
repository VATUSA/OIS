import {useSyncExternalStore} from "react";

/** Pages visited this session, newest first — the sidebar chrome row's history list. */
export type RecentPage = { path: string; label: string };

const MAX = 8;
let recent: RecentPage[] = [];
const listeners = new Set<() => void>();

export function recordVisit(page: RecentPage) {
  if (recent[0]?.path === page.path) return;
  recent = [page, ...recent.filter((p) => p.path !== page.path)].slice(0, MAX);
  listeners.forEach((l) => l());
}

export function useRecentPages(): RecentPage[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => recent,
  );
}

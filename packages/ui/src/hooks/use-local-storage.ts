import * as React from "react";

/**
 * A per-device UI preference persisted in localStorage (sidebar collapsed, a remembered tab).
 * Storage can be missing or throw (private mode, blocked site data) — the value then simply lives in
 * memory for the session. Not for anything that must sync across devices: use server preferences.
 */
export function useLocalStorage<T>(key: string, fallback: T): [T, (next: T) => void] {
  const [value, setValue] = React.useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      return raw == null ? fallback : (JSON.parse(raw) as T);
    } catch {
      return fallback;
    }
  });

  const set = React.useCallback(
    (next: T) => {
      setValue(next);
      try {
        window.localStorage.setItem(key, JSON.stringify(next));
      } catch {
        /* storage unavailable — keep the in-memory value */
      }
    },
    [key],
  );

  return [value, set];
}

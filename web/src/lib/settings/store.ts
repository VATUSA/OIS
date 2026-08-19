import {useCallback} from "react";
import {useQueryClient} from "@tanstack/react-query";

import {usePreferences, useSavePreferences} from "@/lib/preferences";
import {SETTINGS_NAMESPACE, type SettingsBlob} from "./registry";

const QUERY_KEY = ["preferences", SETTINGS_NAMESPACE];

/** The whole settings blob for the current user (React Query; `null`/loading before it arrives). */
export function useSettings() {
  return usePreferences<SettingsBlob>(SETTINGS_NAMESPACE);
}

/**
 * Read + write one user setting, backed by the DB preferences API (namespace "settings"). Falls back
 * to `fallback` while the blob loads or when unset / signed out. `setValue` merges the key into the
 * blob and PUTs it, updating the query cache optimistically so the UI (and maps) react instantly.
 */
export function useSetting<T>(
  key: string,
  fallback: T,
): { value: T; setValue: (v: T) => void; isLoading: boolean } {
  const { data, isLoading } = useSettings();
  const save = useSavePreferences<SettingsBlob>(SETTINGS_NAMESPACE);
  const queryClient = useQueryClient();

  const stored = data?.[key];
  const value = stored === undefined ? fallback : (stored as T);

  const setValue = useCallback(
    (v: T) => {
      const current = queryClient.getQueryData<SettingsBlob>(QUERY_KEY) ?? {};
      const next = { ...current, [key]: v };
      queryClient.setQueryData(QUERY_KEY, next); // optimistic — instant UI update
      save.mutate(next);
    },
    [queryClient, save, key],
  );

  return { value, setValue, isLoading };
}

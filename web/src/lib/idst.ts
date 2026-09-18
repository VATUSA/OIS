import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";
import {usePreferences, useSavePreferences} from "./preferences";

export type IdstFlight = components["schemas"]["IdstFlight"];
export type IdstResponse = components["schemas"]["IdstResponse"];
export type IdstScope = { airports: string[]; tracons: string[]; artccs: string[] };

const SCOPE_NS = "idst";
const EMPTY_SCOPE: IdstScope = { airports: [], tracons: [], artccs: [] };

export function scopeIsEmpty(s: IdstScope): boolean {
  return s.airports.length + s.tracons.length + s.artccs.length === 0;
}

/**
 * The user's IDST scope (airports/TRACONs/ARTCCs), persisted server-side so it follows the account.
 * `setScope` does nothing until the stored scope has loaded, so an edit can't overwrite it.
 */
export function useIdstScope() {
  const prefs = usePreferences<IdstScope>(SCOPE_NS);
  const save = useSavePreferences<IdstScope>(SCOPE_NS);
  const scope: IdstScope = { ...EMPTY_SCOPE, ...(prefs.data ?? {}) };
  const setScope = (s: IdstScope) => {
    if (prefs.isSuccess) save.mutate(s);
  };
  return { scope, setScope };
}

/** FCA-metered ground departures in scope, split into unscheduled / released. Polls while in scope. */
export function useIdst(scope: IdstScope) {
  return useQuery({
    queryKey: ["idst", scope],
    enabled: !scopeIsEmpty(scope),
    // Release/FCA changes arrive instantly via the websocket; the poll only refreshes the drifting
    // advisory EDCTs and is the fallback when the socket is down.
    refetchInterval: 30_000,
    queryFn: async (): Promise<IdstResponse> => {
      const { data, error } = await ois.GET("/api/v1/flow/idst", {
        params: {
          query: {
            airports: scope.airports.join(","),
            tracons: scope.tracons.join(","),
            artccs: scope.artccs.join(","),
          },
        },
      });
      if (error || !data) throw new Error("failed to load IDST");
      return data;
    },
  });
}

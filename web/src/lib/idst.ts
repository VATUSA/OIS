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

/** The user's IDST scope (airports/TRACONs/ARTCCs), persisted server-side so it follows the account. */
export function useIdstScope() {
  const { data } = usePreferences<IdstScope>(SCOPE_NS);
  const save = useSavePreferences<IdstScope>(SCOPE_NS);
  const scope: IdstScope = { ...EMPTY_SCOPE, ...(data ?? {}) };
  return { scope, setScope: (s: IdstScope) => save.mutate(s) };
}

/** FCA-metered ground departures in scope, split into unscheduled / released. Polls while in scope. */
export function useIdst(scope: IdstScope) {
  return useQuery({
    queryKey: ["idst", scope],
    enabled: !scopeIsEmpty(scope),
    refetchInterval: 15_000,
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

import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

/** An ATC facility (ARTCC/center or TRACON/approach) + the airports it covers. */
export type FlowFacility = components["schemas"]["FlowFacility"];

/**
 * The facility directory (ARTCCs + TRACONs → member airports), for scoping dashboard widgets to a whole
 * facility. Refreshed daily server-side, so a long staleTime is fine.
 */
export function useFacilityDirectory() {
  return useQuery({
    queryKey: ["flow-facilities"],
    queryFn: async (): Promise<FlowFacility[]> => {
      const { data, error } = await ois.GET("/api/v1/flow/facilities");
      if (error || !data) throw new Error("failed to load facilities");
      return data;
    },
    staleTime: 24 * 3_600_000,
  });
}

/** Member airport ICAOs for a facility id (empty when unknown / not loaded yet). */
export function facilityAirports(dir: FlowFacility[] | undefined, id: string): string[] {
  return dir?.find((f) => f.id === id)?.airports ?? [];
}

/** Human label for a facility kind. */
export const facilityKindLabel = (kind: string) => (kind === "artcc" ? "Center" : "Approach");

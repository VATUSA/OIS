import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type PublicBoard = components["schemas"]["PublicBoard"];
export type PublicRestriction = components["schemas"]["PublicRestriction"];
export type PublicGroundStop = components["schemas"]["PublicGroundStop"];
export type PublicGdp = components["schemas"]["PublicGdp"];
export type PublicProgram = components["schemas"]["PublicProgram"];
export type FlightAdvisory = components["schemas"]["FlightAdvisory"];

/** Everything currently affecting one flight (by callsign). Public, no auth. */
export function usePublicFlight(callsign: string | null) {
  return useQuery({
    queryKey: ["public-flight", callsign],
    enabled: !!callsign,
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/public/flight/{callsign}", {
        params: { path: { callsign: callsign! } },
      });
      if (error || !data) throw new Error("failed to look up flight");
      return data;
    },
    refetchInterval: 30_000,
  });
}

/** All active TMIs (ground stops, GDPs, restrictions, rate programs). Public, no auth. */
export function usePublicBoard() {
  return useQuery({
    queryKey: ["public-board"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/public/board");
      if (error || !data) throw new Error("failed to load advisories");
      return data;
    },
    refetchInterval: 30_000,
  });
}

import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type PublicBoard = components["schemas"]["PublicBoard"];
export type PublicRestriction = components["schemas"]["PublicRestriction"];
export type PublicGroundStop = components["schemas"]["PublicGroundStop"];
export type PublicGdp = components["schemas"]["PublicGdp"];
export type PublicProgram = components["schemas"]["PublicProgram"];

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

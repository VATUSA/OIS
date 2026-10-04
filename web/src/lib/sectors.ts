import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type SectorVolume = components["schemas"]["SectorVolumeBody"];

/**
 * Every ATC sector volume (#594), for the admin sector map (#602). The set changes only when someone
 * runs the offline importer, so it is fetched once and kept.
 */
export function useSectors() {
  return useQuery({
    queryKey: ["airspace-sectors"],
    queryFn: async (): Promise<SectorVolume[]> => {
      const { data, error } = await ois.GET("/api/v1/flow/airspace/sectors");
      if (error || !data) throw new Error("failed to load sector volumes");
      return data;
    },
    staleTime: Infinity,
  });
}

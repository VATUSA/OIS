import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type AadcBucket = components["schemas"]["AadcBucket"];
export type AadcResponse = components["schemas"]["AadcResponse"];

export type AadcBucketMin = 15 | 30 | 60;
export type AadcDimension = "status" | "category" | "carrier" | "afix";

export const AADC_DIMENSIONS: { value: AadcDimension; label: string }[] = [
  { value: "status", label: "Status" },
  { value: "category", label: "Aircraft category" },
  { value: "carrier", label: "Carrier" },
  { value: "afix", label: "Arrival fix" },
];

/** Bucketed arrival demand for an airport, refreshed on the same cadence as the live flow board. */
export function useAadc(icao: string | null, bucketMin: AadcBucketMin) {
  return useQuery({
    queryKey: ["aadc", icao, bucketMin],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/flow/{icao}/aadc", {
        params: { path: { icao: icao! }, query: { bucket_min: bucketMin } },
      });
      if (error || !data) throw new Error("failed to load AADC demand");
      return data;
    },
    enabled: !!icao,
    refetchInterval: 20_000,
  });
}

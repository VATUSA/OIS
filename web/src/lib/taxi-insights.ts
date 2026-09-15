import {keepPreviousData, useQuery} from "@tanstack/react-query";

import {ois} from "./api";

export type TaxiInsightsFilters = {
  airport?: string;
  gateId?: string;
  aircraft?: string;
  runway?: string;
  from?: string;
  to?: string;
  includeOutliers?: boolean;
};

export function useTaxiObservations(
  page: number,
  pageSize: number,
  filters: TaxiInsightsFilters,
) {
  return useQuery({
    queryKey: ["taxi-observations", page, pageSize, filters],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/stats/taxi/observations", {
        params: {
          query: {
            airport: filters.airport || undefined,
            gate_id: filters.gateId || undefined,
            aircraft: filters.aircraft || undefined,
            runway: filters.runway || undefined,
            from: filters.from || undefined,
            to: filters.to || undefined,
            include_outliers: filters.includeOutliers,
            page,
            page_size: pageSize,
          },
        },
      });
      if (error || !data) throw new Error("failed to load taxi observations");
      return data;
    },
    placeholderData: keepPreviousData,
  });
}

export function useTaxiEstimates(
  page: number,
  pageSize: number,
  filters: TaxiInsightsFilters & { fallbackTier?: string },
) {
  return useQuery({
    queryKey: ["taxi-estimates", page, pageSize, filters],
    enabled: Boolean(filters.airport),
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/stats/taxi/estimates", {
        params: {
          query: {
            airport: filters.airport!,
            gate_id: filters.gateId || undefined,
            aircraft: filters.aircraft || undefined,
            runway: filters.runway || undefined,
            from: filters.from || undefined,
            to: filters.to || undefined,
            include_outliers: filters.includeOutliers,
            fallback_tier: filters.fallbackTier || undefined,
            page,
            page_size: pageSize,
          },
        },
      });
      if (error || !data) throw new Error("failed to load taxi estimates");
      return data;
    },
    placeholderData: keepPreviousData,
  });
}

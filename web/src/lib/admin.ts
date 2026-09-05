import {keepPreviousData, useQuery} from "@tanstack/react-query";

import {ois} from "./api";

export type AuditFilters = { q?: string; from?: string; to?: string };

export function useAuditLog(page = 1, pageSize = 50, filters: AuditFilters = {}) {
  const q = filters.q?.trim() || undefined;
  const from = filters.from || undefined;
  const to = filters.to || undefined;
  return useQuery({
    queryKey: ["audit", page, pageSize, q, from, to],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/audit", {
        params: { query: { page, page_size: pageSize, q, from, to } },
      });
      if (error || !data) throw new Error("failed to load audit log");
      return data;
    },
    // Keep the current page on screen while the next one loads (no flash to "Loading…").
    placeholderData: keepPreviousData,
  });
}

export function useFacilities() {
  return useQuery({
    queryKey: ["facilities"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/facilities");
      if (error || !data) throw new Error("failed to load facilities");
      return data;
    },
  });
}

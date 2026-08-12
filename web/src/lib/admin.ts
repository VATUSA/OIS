import {useQuery} from "@tanstack/react-query";

import {ois} from "./api";

export function useAuditLog(pageSize = 8) {
  return useQuery({
    queryKey: ["audit", pageSize],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/audit", {
        params: { query: { page_size: pageSize } },
      });
      if (error || !data) throw new Error("failed to load audit log");
      return data;
    },
  });
}

export function useServiceAccounts() {
  return useQuery({
    queryKey: ["service-accounts"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/service-accounts");
      if (error || !data) throw new Error("failed to load service accounts");
      return data;
    },
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

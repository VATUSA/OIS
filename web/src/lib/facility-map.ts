import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type FacilityMapConfig = components["schemas"]["FacilityMapConfigBody"];
export type UpsertFacilityMapConfig = components["schemas"]["UpsertFacilityMapConfigRequest"];

const key = (id: string) => ["facility-map-config", id];

/** A facility's map color-rule config (public read; `editable` reflects the caller's scope). */
export function useFacilityMapConfig(id: string | null) {
  return useQuery({
    queryKey: key(id ?? ""),
    queryFn: async (): Promise<FacilityMapConfig> => {
      const { data, error } = await ois.GET("/api/v1/facility-map/{id}/config", {
        params: { path: { id: id! } },
      });
      if (error || !data) throw new Error("failed to load facility map config");
      return data;
    },
    enabled: !!id,
    staleTime: 30_000,
  });
}

/** Save a facility's color rules (requires flow.facility_map.update for that facility). */
export function useSaveFacilityMapConfig(id: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertFacilityMapConfig): Promise<FacilityMapConfig> => {
      const { data, error } = await ois.PUT("/api/v1/facility-map/{id}/config", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => queryClient.setQueryData(key(id), data),
    onError: () => toast.error("Couldn’t save color rules"),
  });
}

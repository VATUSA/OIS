import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type FacilityDocument = components["schemas"]["FacilityDocumentBody"];
export type UpsertFacilityDocument = components["schemas"]["UpsertFacilityDocumentRequest"];

/** A facility's configured reference documents (SOPs/LOAs/etc.). */
export function useFacilityDocuments(facilityId: string | null) {
  return useQuery({
    queryKey: ["facility-documents", facilityId],
    enabled: !!facilityId,
    queryFn: async (): Promise<FacilityDocument[]> => {
      const { data, error } = await ois.GET("/api/v1/facilities/{facility_id}/documents", {
        params: { path: { facility_id: facilityId! } },
      });
      if (error || !data) throw new Error("failed to load facility documents");
      return data;
    },
  });
}

export function useCreateFacilityDocument(facilityId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: UpsertFacilityDocument): Promise<FacilityDocument> => {
      const { data, error } = await ois.POST("/api/v1/facilities/{facility_id}/documents", {
        params: { path: { facility_id: facilityId } },
        body,
      });
      if (error || !data) throw new Error("create facility document failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["facility-documents", facilityId] }),
  });
}

export function useUpdateFacilityDocument(facilityId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      id: string;
      body: UpsertFacilityDocument;
    }): Promise<FacilityDocument> => {
      const { data, error } = await ois.PUT("/api/v1/facilities/{facility_id}/documents/{id}", {
        params: { path: { facility_id: facilityId, id: input.id } },
        body: input.body,
      });
      if (error || !data) throw new Error("update facility document failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["facility-documents", facilityId] }),
  });
}

export function useDeleteFacilityDocument(facilityId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/facilities/{facility_id}/documents/{id}", {
        params: { path: { facility_id: facilityId, id } },
      });
      if (error) throw new Error("delete facility document failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["facility-documents", facilityId] }),
  });
}

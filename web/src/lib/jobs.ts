import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type JobStatus = components["schemas"]["JobStatus"];

/** Background-job statuses for the admin viewer. Polls so the table stays live. */
export function useJobs() {
  return useQuery({
    queryKey: ["admin-jobs"],
    queryFn: async (): Promise<JobStatus[]> => {
      const { data, error } = await ois.GET("/api/v1/admin/jobs");
      if (error || !data) throw new Error("failed to load background jobs");
      return data;
    },
    refetchInterval: 5000,
  });
}

/** Trigger a triggerable job to run now. */
export function useRunJob() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) => {
      const { error } = await ois.POST("/api/v1/admin/jobs/{name}/run", {
        params: { path: { name } },
      });
      if (error) throw new Error("failed to trigger job");
    },
    // Give the job a beat to start, then refresh so its "running" state shows.
    onSuccess: () =>
      setTimeout(() => qc.invalidateQueries({ queryKey: ["admin-jobs"] }), 400),
  });
}

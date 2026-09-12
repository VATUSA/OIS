import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type JobStatus = components["schemas"]["JobStatus"];

const JOBS_KEY = ["admin-jobs"] as const;

/** Idle poll cadence — plenty responsive for a page that mostly shows completed/scheduled state. */
const IDLE_POLL_MS = 5000;
/** Fast poll cadence while any task is actively running, so completion appears within about a
 * second instead of waiting out the idle cadence. */
const ACTIVE_POLL_MS = 1000;

/** The poll interval for the given job list: fast while anything is running, idle otherwise. */
export function jobsPollInterval(jobs: JobStatus[] | undefined): number {
  return jobs?.some((j) => j.running) ? ACTIVE_POLL_MS : IDLE_POLL_MS;
}

/** Background-job statuses for the admin viewer. Polls so the table stays live, faster while a
 * task is actively running. */
export function useJobs() {
  return useQuery({
    queryKey: JOBS_KEY,
    queryFn: async (): Promise<JobStatus[]> => {
      const { data, error } = await ois.GET("/api/v1/admin/jobs");
      if (error || !data) throw new Error("failed to load background jobs");
      return data;
    },
    refetchInterval: (query) => jobsPollInterval(query.state.data),
  });
}

/** Trigger a triggerable job to run now. Marks it "running" optimistically so the row updates with
 * no visible delay; the following fast poll (see `useJobs`) picks up the real outcome within ~1s. */
export function useRunJob() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (name: string) => {
      const { error } = await ois.POST("/api/v1/admin/jobs/{name}/run", {
        params: { path: { name } },
      });
      if (error) throw new Error("failed to trigger job");
    },
    onMutate: async (name: string) => {
      await qc.cancelQueries({ queryKey: JOBS_KEY });
      const previous = qc.getQueryData<JobStatus[]>(JOBS_KEY);
      qc.setQueryData<JobStatus[]>(JOBS_KEY, (jobs) =>
        jobs?.map((j) => (j.name === name ? { ...j, running: true } : j)),
      );
      return { previous };
    },
    onError: (_error, _name, context) => {
      if (context?.previous) qc.setQueryData(JOBS_KEY, context.previous);
      toast.error("Couldn't trigger that task");
    },
    onSettled: () => qc.invalidateQueries({ queryKey: JOBS_KEY }),
  });
}

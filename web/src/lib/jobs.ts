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

/** Optimistically mark one job as running, leaving every other row untouched. */
export function withJobRunning(jobs: JobStatus[] | undefined, name: string): JobStatus[] | undefined {
  return jobs?.map((j) => (j.name === name ? { ...j, running: true } : j));
}

/** Restore one job to a prior snapshot, leaving every other row untouched — in particular, a
 * different job's own concurrent optimistic update (e.g. from a second in-flight `useRunJob` call)
 * must survive this, since each `JobRow` triggers independently against the same cached list. */
export function withJobRestored(
  jobs: JobStatus[] | undefined,
  name: string,
  restored: JobStatus,
): JobStatus[] | undefined {
  return jobs?.map((j) => (j.name === name ? restored : j));
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
 * no visible delay. Deliberately does NOT invalidate/refetch on success: the trigger endpoint only
 * wakes the job loop (`JobRegistry::trigger` calls `notify_one()` and returns before the loop calls
 * `begin()`), so an immediate refetch can race ahead of that and overwrite the optimistic
 * `running: true` with stale `running: false` — reverting the row and dropping the poll back to the
 * slow cadence. Leaving the optimistic value in place lets `jobsPollInterval` pick the fast cadence
 * immediately, and the next poll tick (≤1s later) picks up the real outcome without racing. */
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
      const previousJob = qc.getQueryData<JobStatus[]>(JOBS_KEY)?.find((j) => j.name === name);
      qc.setQueryData<JobStatus[]>(JOBS_KEY, (jobs) => withJobRunning(jobs, name));
      return { previousJob };
    },
    onError: (_error, name, context) => {
      if (context?.previousJob) {
        const restored = context.previousJob;
        qc.setQueryData<JobStatus[]>(JOBS_KEY, (jobs) => withJobRestored(jobs, name, restored));
      }
      toast.error("Couldn't trigger that task");
    },
  });
}

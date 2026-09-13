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

/** Apply `updater` to one job, leaving every other row untouched — in particular, a different job's
 * own concurrent optimistic update (each `JobRow` triggers independently against the same cached
 * list) must survive, and any fields a poll has since refreshed on *this* job (runs, last_ok,
 * last_detail, ...) must survive too, so callers should patch only the field(s) they own rather than
 * replace the whole row. */
export function updateJob(
  jobs: JobStatus[] | undefined,
  name: string,
  updater: (job: JobStatus) => JobStatus,
): JobStatus[] | undefined {
  return jobs?.map((j) => (j.name === name ? updater(j) : j));
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

/** Trigger `name` to run now. Marks it "running" optimistically so the row updates with no visible
 * delay. Deliberately does NOT invalidate/refetch on success: the trigger endpoint only wakes the
 * job loop (`JobRegistry::trigger` calls `notify_one()` and returns before the loop calls `begin()`),
 * so an immediate refetch can race ahead of that and overwrite the optimistic `running: true` with
 * stale `running: false` — reverting the row and dropping the poll back to the slow cadence. Leaving
 * the optimistic value in place lets `jobsPollInterval` pick the fast cadence immediately, and the
 * next poll tick (≤1s later) picks up the real outcome without racing.
 *
 * `scope: { id: name }` serializes repeated calls for the *same* job one at a time (TanStack Query
 * queues same-scope mutations rather than running them concurrently) — without it, a fast
 * double-click could fire two overlapping mutations whose onMutate/onError interleave and leave the
 * row's optimistic state wrong until the next poll. */
export function useRunJob(name: string) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    scope: { id: `run-job:${name}` },
    mutationFn: async () => {
      const { error } = await ois.POST("/api/v1/admin/jobs/{name}/run", {
        params: { path: { name } },
      });
      if (error) throw new Error("failed to trigger job");
    },
    onMutate: async () => {
      await qc.cancelQueries({ queryKey: JOBS_KEY });
      const previousJob = qc.getQueryData<JobStatus[]>(JOBS_KEY)?.find((j) => j.name === name);
      qc.setQueryData<JobStatus[]>(JOBS_KEY, (jobs) =>
        updateJob(jobs, name, (j) => ({ ...j, running: true })),
      );
      return { previousJob };
    },
    // Restores only the `running` flag this mutation itself set, not the whole previousJob
    // snapshot — a poll landing between onMutate and onError may have already refreshed runs/
    // last_ok/last_detail for this same job (e.g. the trigger actually succeeded server-side but the
    // client-perceived request failed), and that fresher data must survive the rollback.
    onError: (_error, _vars, context) => {
      if (context?.previousJob) {
        const runningWas = context.previousJob.running;
        qc.setQueryData<JobStatus[]>(JOBS_KEY, (jobs) =>
          updateJob(jobs, name, (j) => ({ ...j, running: runningWas })),
        );
      }
      toast.error("Couldn't trigger that task");
    },
  });
}

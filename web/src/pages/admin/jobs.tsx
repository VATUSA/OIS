import {Badge, Button, Card, CardContent} from "@ois/ui";

import {timeAgo} from "@/lib/time";
import {type JobStatus, useJobs, useRunJob} from "@/lib/jobs";

/** A short human interval like "15m" / "24h" from a seconds value. */
function formatInterval(secs: number | null | undefined): string {
  if (!secs) return "continuous";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h`;
  return `${Math.round(secs / 86400)}d`;
}

function StatusBadge({ job }: { job: JobStatus }) {
  if (job.running) return <Badge variant="secondary">Running…</Badge>;
  if (job.last_ok == null) return <Badge variant="outline">Never run</Badge>;
  return job.last_ok ? (
    <Badge variant="secondary">OK</Badge>
  ) : (
    <Badge variant="destructive">Failed</Badge>
  );
}

function JobRow({ job }: { job: JobStatus }) {
  const run = useRunJob();
  const lastRun =
    job.last_finished_ms > 0 ? timeAgo(new Date(job.last_finished_ms).toISOString()) : "—";

  return (
    <tr className="border-t align-top">
      <td className="py-2 pr-3">
        <div className="font-medium">{job.name}</div>
        <div className="text-xs text-muted-foreground">{job.description}</div>
      </td>
      <td className="py-2 pr-3">
        <StatusBadge job={job} />
      </td>
      <td className="py-2 pr-3 whitespace-nowrap tabular-nums text-muted-foreground">{lastRun}</td>
      <td className="py-2 pr-3 text-xs text-muted-foreground">{job.last_detail ?? "—"}</td>
      <td className="py-2 pr-3 tabular-nums text-muted-foreground">
        {formatInterval(job.interval_secs)}
      </td>
      <td className="py-2 pr-3 tabular-nums text-muted-foreground">{job.runs}</td>
      <td className="py-2 text-right">
        {job.triggerable ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2"
            disabled={job.running || run.isPending}
            onClick={() => run.mutate(job.name)}
          >
            Run now
          </Button>
        ) : (
          <span className="text-xs text-muted-foreground">auto</span>
        )}
      </td>
    </tr>
  );
}

export function AdminJobs() {
  const jobs = useJobs();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Background Tasks</h1>
        <p className="text-muted-foreground">
          When each background job last ran, its outcome, and a manual trigger for the ones that
          support it.
        </p>
      </div>
      <Card>
        <CardContent className="overflow-x-auto pt-6">
          {jobs.data ? (
            jobs.data.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                No background jobs are registered.
              </p>
            ) : (
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Task</th>
                    <th className="pb-2 pr-3 font-medium">Status</th>
                    <th className="pb-2 pr-3 font-medium">Last run</th>
                    <th className="pb-2 pr-3 font-medium">Detail</th>
                    <th className="pb-2 pr-3 font-medium">Interval</th>
                    <th className="pb-2 pr-3 font-medium">Runs</th>
                    <th className="pb-2" />
                  </tr>
                </thead>
                <tbody>
                  {jobs.data.map((job) => (
                    <JobRow key={job.name} job={job} />
                  ))}
                </tbody>
              </table>
            )
          ) : jobs.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load background tasks.
            </p>
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

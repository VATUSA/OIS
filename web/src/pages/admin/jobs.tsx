import {Button, type DataColumn, DataTable, MetricCard, StatusPill} from "@ois/ui";
import {Activity, AlertTriangle, Clock, Database, Hash, ListChecks, Loader2, Repeat} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {jobState} from "@/lib/status";
import {timeAgo} from "@/lib/time";
import {type JobStatus, useJobs, useRunJob} from "@/lib/jobs";
import {useStorageForecast} from "@/lib/stats";

/** A short human interval like "15m" / "24h" from a seconds value. */
function formatInterval(secs: number | null | undefined): string {
  if (!secs) return "continuous";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h`;
  return `${Math.round(secs / 86400)}d`;
}

/** A human byte size, e.g. `4.2 GB`. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let n = bytes / 1024;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}

/**
 * Current `stats` schema disk usage and a naive growth projection — gives the compaction jobs
 * size context. The projection is a deliberately simple upper bound: it assumes the current daily
 * ingest rate holds and does not model the compaction ladder's ongoing thinning.
 */
function StorageMetric() {
  const forecast = useStorageForecast();
  if (!forecast.data) return null;
  const f = forecast.data;
  return (
    <MetricCard
      label="Stats storage"
      icon={Database}
      value={formatBytes(f.total_bytes)}
      sub={
        <span className="font-mono">
          +{formatBytes(f.daily_growth_bytes)}/day · 30d {formatBytes(f.projected_30d_bytes)} · 90d{" "}
          {formatBytes(f.projected_90d_bytes)}
        </span>
      }
    />
  );
}

function RunNow({ job }: { job: JobStatus }) {
  const run = useRunJob(job.name);
  if (!job.triggerable) return <span className="text-xs text-ink-3">auto</span>;
  return (
    <Button size="sm" variant="ghost" disabled={job.running || run.isPending} onClick={() => run.mutate()}>
      Run now
    </Button>
  );
}

const COLUMNS: DataColumn<JobStatus>[] = [
  {
    accessorKey: "name",
    header: "Task",
    icon: ListChecks,
    cell: (c) => (
      <div className="min-w-48">
        <div className="font-semibold">{c.row.original.name}</div>
        <div className="text-xs text-ink-2">{c.row.original.description}</div>
      </div>
    ),
  },
  {
    id: "status",
    accessorFn: (j) => jobState(j).label,
    header: "Status",
    icon: Activity,
    cell: (c) => {
      const s = jobState(c.row.original);
      return <StatusPill tone={s.tone}>{s.label}</StatusPill>;
    },
  },
  {
    accessorKey: "last_finished_ms",
    header: "Last run",
    icon: Clock,
    mono: true,
    cell: (c) => {
      const ms = c.getValue<number>();
      return (
        <span className="whitespace-nowrap text-ink-2">
          {ms > 0 ? timeAgo(new Date(ms).toISOString()) : "—"}
        </span>
      );
    },
  },
  {
    accessorKey: "last_detail",
    header: "Detail",
    enableSorting: false,
    cell: (c) => (
      <span className={c.row.original.last_ok === false ? "text-xs font-semibold text-danger" : "text-xs text-ink-2"}>
        {c.getValue<string | null>() ?? "—"}
      </span>
    ),
  },
  {
    accessorKey: "interval_secs",
    header: "Interval",
    icon: Repeat,
    mono: true,
    cell: (c) => <span className="text-ink-2">{formatInterval(c.getValue<number | null>())}</span>,
  },
  {
    accessorKey: "runs",
    header: "Runs",
    icon: Hash,
    mono: true,
    align: "right",
  },
  {
    id: "actions",
    header: "",
    enableSorting: false,
    align: "right",
    cell: (c) => <RunNow job={c.row.original} />,
  },
];

const SUBTITLE =
  "When each background job last ran, its outcome, and a manual trigger for the ones that support it.";

export function AdminJobs() {
  const jobs = useJobs();
  const list = jobs.data ?? [];
  const running = list.filter((j) => j.running).length;
  const failing = list.filter((j) => !j.running && j.last_ok === false).length;

  usePageHeader({ subtitle: SUBTITLE, count: jobs.data?.length ?? null });

  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard label="Tasks" icon={ListChecks} value={jobs.data ? list.length : "—"} />
        <MetricCard
          label="Running"
          icon={Loader2}
          value={jobs.data ? running : "—"}
          tone={running > 0 ? "brand" : undefined}
        />
        <MetricCard
          label="Failing"
          icon={AlertTriangle}
          value={jobs.data ? failing : "—"}
          tone={failing > 0 ? "bad" : undefined}
        />
        <StorageMetric />
      </div>

      <DataTable
        label="Background tasks"
        columns={COLUMNS}
        data={list}
        getRowId={(j) => j.name}
        rowCap={50}
        isLoading={jobs.isLoading}
        isError={jobs.isError}
        onRetry={() => jobs.refetch()}
        empty="No background jobs are registered."
      />
    </div>
  );
}

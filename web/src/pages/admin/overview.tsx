import {Link} from "@tanstack/react-router";
import {buttonVariants, EmptyState, MetricCard, QueryState, Sparkline, StatusPill} from "@ois/ui";
import {Activity, ArrowRight, LayoutDashboard, ScrollText, UserPlus} from "lucide-react";

import {AuditTable} from "@/components/admin/audit-table";
import {type DailySeries, useAdminSummary, useAuditLog, weekOverWeek} from "@/lib/admin";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";

function SeriesCard({ label, icon, series }: { label: string; icon: typeof ScrollText; series: DailySeries }) {
  const trend = weekOverWeek(series);
  const tone = trend.direction === "up" ? "good" : trend.direction === "down" ? "bad" : "warn";
  return (
    <MetricCard
      label={label}
      icon={icon}
      value={series.total}
      sub="last 30 days · vs prior week"
      trend={trend}
      sparkline={<Sparkline values={series.points.map((p) => p.count)} tone={tone} label={`${label}, last 30 days`} />}
    />
  );
}

/**
 * The Admin landing (the console dashboard): metric cards for the sections the user can read, then
 * the most recent audit activity.
 */
export function AdminOverview() {
  const { data: me } = useMe();
  const summary = useAdminSummary();
  const canAudit = hasPermission(me, "audit.logs.read");
  const audit = useAuditLog(1, 10);
  const s = summary.data;
  const hasMetrics = !!s && (s.audit_events || s.new_users || s.jobs);

  return (
    <div className="flex flex-col gap-6">
      <QueryState isLoading={summary.isLoading} isError={summary.isError} onRetry={() => summary.refetch()}>
        {hasMetrics ? (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {s.audit_events && <SeriesCard label="Audit events" icon={ScrollText} series={s.audit_events} />}
            {s.new_users && <SeriesCard label="New users" icon={UserPlus} series={s.new_users} />}
            {s.jobs && (
              <MetricCard
                label="Background jobs"
                icon={Activity}
                value={s.jobs.total}
                tone={s.jobs.failing > 0 ? "bad" : undefined}
                sub={
                  <StatusPill tone={s.jobs.failing > 0 ? "bad" : "good"} dot>
                    {s.jobs.failing > 0 ? `${s.jobs.failing} failing` : "All healthy"}
                  </StatusPill>
                }
              />
            )}
          </div>
        ) : (
          <EmptyState icon={LayoutDashboard} title="Admin">
            Pick a section from the sidebar.
          </EmptyState>
        )}
      </QueryState>

      {canAudit && (
        <section className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <h2 className="text-xl font-bold">Recent activity</h2>
            <Link to="/admin/audit" className={buttonVariants({ variant: "ghost", size: "sm" })}>
              View all
              <ArrowRight />
            </Link>
          </div>
          <AuditTable items={audit.data?.items ?? []} isLoading={audit.isLoading} isError={audit.isError} />
        </section>
      )}
    </div>
  );
}

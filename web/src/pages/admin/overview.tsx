import type {ReactNode} from "react";
import {Link} from "@tanstack/react-router";
import {Building2, KeyRound, type LucideIcon, ScrollText, ShieldCheck,} from "lucide-react";
import {Card, CardContent, CardHeader, CardTitle} from "@ois/ui";

import {ActivityList} from "@/components/admin/activity";
import {useAuditLog, useFacilities, useServiceAccounts} from "@/lib/admin";

function Stat({
  label,
  value,
  sub,
  icon: Icon,
}: {
  label: string;
  value: ReactNode;
  sub: string;
  icon: LucideIcon;
}) {
  return (
    <Card>
      <CardContent className="p-5">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            {label}
          </span>
          <span className="flex size-8 items-center justify-center rounded-md bg-muted text-muted-foreground">
            <Icon className="size-4" />
          </span>
        </div>
        <div className="mt-2 text-3xl font-bold">{value}</div>
        <div className="text-xs text-muted-foreground">{sub}</div>
      </CardContent>
    </Card>
  );
}

export function AdminOverview() {
  const facilities = useFacilities();
  const audit = useAuditLog(8);
  const serviceAccounts = useServiceAccounts();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
        <p className="text-muted-foreground">
          Server administration at a glance.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat
          label="Facilities"
          value={facilities.data?.length ?? "—"}
          sub="ARTCCs"
          icon={Building2}
        />
        <Stat
          label="Audit Entries"
          value={audit.data?.total ?? "—"}
          sub="recorded"
          icon={ScrollText}
        />
        <Stat
          label="Service Accounts"
          value={serviceAccounts.data?.length ?? "—"}
          sub="machine clients"
          icon={KeyRound}
        />
        <Stat label="Access" value="Admin" sub="your role" icon={ShieldCheck} />
      </div>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Recent Activity</CardTitle>
          <Link
            to="/admin/audit"
            className="text-sm text-primary hover:underline"
          >
            View all →
          </Link>
        </CardHeader>
        <CardContent>
          {audit.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load activity.
            </p>
          ) : audit.data ? (
            <ActivityList items={audit.data.items} />
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Loading…
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

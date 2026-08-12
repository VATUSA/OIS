import {Card, CardContent} from "@ois/ui";

import {ActivityList} from "@/components/admin/activity";
import {useAuditLog} from "@/lib/admin";

export function AdminAudit() {
  const audit = useAuditLog(50);

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Audit Log</h1>
        <p className="text-muted-foreground">
          Every recorded administrative action, most recent first.
        </p>
      </div>
      <Card>
        <CardContent className="pt-6">
          {audit.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load the audit log.
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

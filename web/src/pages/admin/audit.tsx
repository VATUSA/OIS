import {useState} from "react";
import {Card, CardContent} from "@ois/ui";

import {ActivityList} from "@/components/admin/activity";
import {Pagination} from "@/components/pagination";
import {useAuditLog} from "@/lib/admin";

const PAGE_SIZE = 50;

export function AdminAudit() {
  const [page, setPage] = useState(1);
  const audit = useAuditLog(page, PAGE_SIZE);

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Audit Log</h1>
        <p className="text-muted-foreground">
          Every recorded administrative action, most recent first.
        </p>
      </div>
      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          {audit.data ? (
            <>
              <ActivityList items={audit.data.items} />
              <Pagination
                page={audit.data.page}
                pageSize={audit.data.page_size}
                total={audit.data.total}
                onPageChange={setPage}
              />
            </>
          ) : audit.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load the audit log.
            </p>
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

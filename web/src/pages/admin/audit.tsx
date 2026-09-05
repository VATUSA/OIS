import {useEffect, useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";

import {AuditTable} from "@/components/admin/audit-table";
import {Pagination} from "@/components/pagination";
import {useAuditLog} from "@/lib/admin";

const PAGE_SIZE = 50;

export function AdminAudit() {
  const [page, setPage] = useState(1);
  // Draft inputs vs. the applied filters that actually drive the query.
  const [qDraft, setQDraft] = useState("");
  const [fromDraft, setFromDraft] = useState("");
  const [toDraft, setToDraft] = useState("");
  const [filters, setFilters] = useState<{ q: string; from: string; to: string }>({
    q: "",
    from: "",
    to: "",
  });

  // Any filter change resets to the first page so results aren't hidden on a later page.
  useEffect(() => {
    setPage(1);
  }, [filters]);

  const apply = () =>
    setFilters({
      q: qDraft,
      // datetime-local yields "YYYY-MM-DDTHH:mm"; append seconds so it parses as RFC 3339.
      from: fromDraft ? `${fromDraft}:00Z` : "",
      to: toDraft ? `${toDraft}:00Z` : "",
    });
  const clear = () => {
    setQDraft("");
    setFromDraft("");
    setToDraft("");
    setFilters({ q: "", from: "", to: "" });
  };
  const hasFilters = Boolean(filters.q || filters.from || filters.to);

  const audit = useAuditLog(page, PAGE_SIZE, filters);

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
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              apply();
            }}
          >
            <label className="flex flex-1 flex-col gap-1 text-xs font-medium text-muted-foreground">
              Search
              <Input
                placeholder="Action, resource, reason, actor name or CID…"
                value={qDraft}
                onChange={(e) => setQDraft(e.target.value)}
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              From
              <Input
                type="datetime-local"
                value={fromDraft}
                onChange={(e) => setFromDraft(e.target.value)}
              />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
              To
              <Input
                type="datetime-local"
                value={toDraft}
                onChange={(e) => setToDraft(e.target.value)}
              />
            </label>
            <Button type="submit" size="sm">
              Search
            </Button>
            {hasFilters && (
              <Button type="button" size="sm" variant="ghost" onClick={clear}>
                Clear
              </Button>
            )}
          </form>
          {audit.data ? (
            <>
              <AuditTable items={audit.data.items} />
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

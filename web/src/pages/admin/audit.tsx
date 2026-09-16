import {useEffect, useState} from "react";
import {Button, FilterBar, Input} from "@ois/ui";
import {Search} from "lucide-react";

import {ZuluDateTime} from "@/components/zulu-datetime";
import {AuditTable} from "@/components/admin/audit-table";
import {usePageHeader} from "@/components/shell/page-meta";
import {useAuditLog} from "@/lib/admin";

const PAGE_SIZE = 50;
const SUBTITLE = "Every recorded administrative action, most recent first.";

export function AdminAudit() {
  const [page, setPage] = useState(1);
  // Draft inputs vs. the applied filters that actually drive the query.
  const [qDraft, setQDraft] = useState("");
  const [fromDraft, setFromDraft] = useState<number | null>(null);
  const [toDraft, setToDraft] = useState<number | null>(null);
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
      from: fromDraft != null ? new Date(fromDraft * 1000).toISOString() : "",
      to: toDraft != null ? new Date(toDraft * 1000).toISOString() : "",
    });
  const clear = () => {
    setQDraft("");
    setFromDraft(null);
    setToDraft(null);
    setFilters({ q: "", from: "", to: "" });
  };
  const hasFilters = Boolean(filters.q || filters.from || filters.to);

  const audit = useAuditLog(page, PAGE_SIZE, filters);

  usePageHeader({ subtitle: SUBTITLE, count: audit.data?.total ?? null });

  return (
    <div className="flex flex-col gap-4">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <FilterBar>
          <div className="relative min-w-56 flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-ink-3" />
            <Input
              aria-label="Search"
              placeholder="Action, resource, reason, actor name or CID…"
              value={qDraft}
              onChange={(e) => setQDraft(e.target.value)}
              className="rounded-full pl-9"
            />
          </div>
          <div className="flex items-center gap-2 text-xs font-semibold text-ink-3">
            From
            <ZuluDateTime label="From" value={fromDraft} onChange={setFromDraft} onClear={() => setFromDraft(null)} />
          </div>
          <div className="flex items-center gap-2 text-xs font-semibold text-ink-3">
            To
            <ZuluDateTime label="To" value={toDraft} onChange={setToDraft} onClear={() => setToDraft(null)} />
          </div>
          <Button type="submit" size="sm">
            Search
          </Button>
          {hasFilters && (
            <Button type="button" size="sm" variant="ghost" onClick={clear}>
              Clear
            </Button>
          )}
        </FilterBar>
      </form>

      <AuditTable
        items={audit.data?.items ?? []}
        isLoading={audit.isLoading}
        isError={audit.isError}
        rowCap={PAGE_SIZE}
        serverPagination={
          audit.data
            ? {
                page: audit.data.page,
                pageSize: audit.data.page_size,
                total: audit.data.total,
                onPageChange: setPage,
              }
            : undefined
        }
      />
    </div>
  );
}

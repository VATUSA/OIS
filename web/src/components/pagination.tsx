import {Button} from "@ois/ui";
import {ChevronLeft, ChevronRight} from "lucide-react";

/**
 * A compact prev/next pager for offset-paginated lists (audit log, etc.). Shows the current range and
 * page count; `page` is 1-based. Renders nothing when everything fits on one page.
 */
export function Pagination({
  page,
  pageSize,
  total,
  onPageChange,
}: {
  page: number;
  pageSize: number;
  total: number;
  onPageChange: (page: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  if (total === 0 || pages <= 1) return null;
  const from = (page - 1) * pageSize + 1;
  const to = Math.min(page * pageSize, total);
  return (
    <div className="flex items-center justify-between gap-4 text-sm">
      <span className="tabular-nums text-muted-foreground">
        {from}–{to} of {total}
      </span>
      <div className="flex items-center gap-2">
        <Button size="sm" variant="secondary" disabled={page <= 1} onClick={() => onPageChange(page - 1)}>
          <ChevronLeft className="size-4" />
          Prev
        </Button>
        <span className="tabular-nums text-muted-foreground">
          Page {page} of {pages}
        </span>
        <Button size="sm" variant="secondary" disabled={page >= pages} onClick={() => onPageChange(page + 1)}>
          Next
          <ChevronRight className="size-4" />
        </Button>
      </div>
    </div>
  );
}

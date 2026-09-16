import * as React from "react";
import {
  type ColumnDef,
  type RowData,
  createSortedRowModel,
  flexRender,
  rowSortingFeature,
  type SortingState,
  sortFns,
  tableFeatures,
  useTable,
} from "@tanstack/react-table";
import {ArrowDown, ArrowUp, ChevronLeft, ChevronRight, ChevronsUpDown, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {Button} from "./button";
import {pageWindow, toggleAll, toggleId} from "./data-table-paging";
import {QueryState} from "./query-state";

// v9 registers features explicitly. The full `sortFns` registry keeps auto-detected sorting for every
// accessor column (#133).
const features = tableFeatures({
  rowSortingFeature,
  sortedRowModel: createSortedRowModel(),
  sortFns,
});

/** A DataTable column: a TanStack column def plus presentation hints. */
export type DataColumn<T extends RowData> = ColumnDef<typeof features, T> & {
  /** Right-align numbers; center for icons/toggles. */
  align?: "left" | "right" | "center";
  /** Render the cell in mono with tabular figures (IDs, counts, times). */
  mono?: boolean;
  /** Icon leading the header label (DESIGN.md icon-led headers). */
  icon?: LucideIcon;
  headerClassName?: string;
  cellClassName?: string;
};

export type ServerPagination = {
  page: number;
  pageSize: number;
  total: number;
  onPageChange: (page: number) => void;
};

export type Selection =
  | { mode: "single"; selected: string | null; onChange: (id: string | null) => void }
  | { mode: "multi"; selected: ReadonlySet<string>; onChange: (ids: Set<string>) => void };

export type DataTableProps<T extends RowData> = {
  columns: DataColumn<T>[];
  data: readonly T[];
  /** Stable row identity — required for selection; defaults to the row index. */
  getRowId?: (row: T, index: number) => string;
  /** Initial sort (uncontrolled) … */
  initialSort?: SortingState;
  /** … or a controlled sort (e.g. persisted by a dashboard widget). */
  sort?: SortingState;
  onSortChange?: (sort: SortingState) => void;
  /** Rows shown before "Show all" (default 10). */
  rowCap?: number;
  /** Client-side page size once expanded (default 50). Ignored with `serverPagination`. */
  pageSize?: number;
  /** The API pages; `data` is the current page. The row cap still applies within the page. */
  serverPagination?: ServerPagination;
  selection?: Selection;
  onRowClick?: (row: T) => void;
  rowClassName?: (row: T) => string | undefined;
  /** Keep the header visible while the table scrolls inside a bounded container. */
  stickyHeader?: boolean;
  hideHeader?: boolean;
  isLoading?: boolean;
  isError?: boolean;
  onRetry?: () => void;
  empty?: React.ReactNode;
  /** Accessible name for the table. */
  label?: string;
  className?: string;
};

const ALIGN = { left: "text-left", right: "text-right", center: "text-center" } as const;

/**
 * The one table (DESIGN.md "Data table"): icon-led headers on `--card`, `--line-soft` row rules,
 * tabular figures, sortable accessor columns, opt-in selection, a default row cap with "Show all",
 * then pagination (client-side, or the API's via `serverPagination`).
 */
export function DataTable<T extends RowData>({
  columns,
  data,
  getRowId,
  initialSort,
  sort,
  onSortChange,
  rowCap = 10,
  pageSize = 50,
  serverPagination,
  selection,
  onRowClick,
  rowClassName,
  stickyHeader = false,
  hideHeader = false,
  isLoading,
  isError,
  onRetry,
  empty = "No rows.",
  label,
  className,
}: DataTableProps<T>) {
  const [ownSort, setOwnSort] = React.useState<SortingState>(initialSort ?? []);
  const sorting = sort ?? ownSort;
  const [expanded, setExpanded] = React.useState(false);
  const [page, setPage] = React.useState(1);

  const table = useTable({
    features,
    data: data as T[],
    columns: columns as ColumnDef<typeof features, T>[],
    getRowId: getRowId ? (row: T, index: number) => getRowId(row, index) : undefined,
    state: { sorting },
    onSortingChange: (updater) => {
      const next = typeof updater === "function" ? updater(sorting) : updater;
      if (onSortChange) onSortChange(next);
      if (sort === undefined) setOwnSort(next);
    },
  });

  const rows = table.getRowModel().rows;
  const win = pageWindow(rows.length, {
    rowCap,
    expanded,
    page: serverPagination ? 1 : page,
    pageSize: serverPagination ? Math.max(rows.length, 1) : pageSize,
  });
  const shown = rows.slice(win.start, win.end);
  const shownIds = shown.map((r) => r.id);

  const multi = selection?.mode === "multi" ? selection : null;
  const single = selection?.mode === "single" ? selection : null;
  const isSelected = (id: string) => (multi ? multi.selected.has(id) : single?.selected === id);
  const allShownSelected = multi != null && shownIds.length > 0 && shownIds.every((id) => multi.selected.has(id));

  const clickable = onRowClick != null || single != null;
  const onRow = (row: (typeof rows)[number]) => {
    if (single) single.onChange(single.selected === row.id ? null : row.id);
    onRowClick?.(row.original);
  };

  const body = (
    <div className={cn("overflow-hidden rounded-md border border-line", className)}>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-sm" aria-label={label}>
          {!hideHeader && (
            <thead className={cn("bg-card", stickyHeader && "sticky top-0 z-10")}>
              {table.getHeaderGroups().map((hg) => (
                <tr key={hg.id} className="border-b border-line">
                  {multi && (
                    <th className="w-9 px-3 py-2">
                      <input
                        type="checkbox"
                        aria-label="Select all shown rows"
                        checked={allShownSelected}
                        onChange={() => multi.onChange(toggleAll(multi.selected, shownIds))}
                        className="size-3.5 accent-brand"
                      />
                    </th>
                  )}
                  {hg.headers.map((h) => {
                    const def = h.column.columnDef as DataColumn<T>;
                    const Icon = def.icon;
                    const canSort = h.column.getCanSort();
                    const sorted = h.column.getIsSorted();
                    return (
                      <th
                        key={h.id}
                        scope="col"
                        aria-sort={sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : undefined}
                        className={cn(
                          "whitespace-nowrap px-3 py-2 text-xs font-semibold text-ink-3",
                          ALIGN[def.align ?? "left"],
                          def.headerClassName,
                        )}
                      >
                        {h.isPlaceholder ? null : (
                          <button
                            type="button"
                            disabled={!canSort}
                            onClick={h.column.getToggleSortingHandler()}
                            className={cn(
                              "inline-flex items-center gap-1.5",
                              canSort ? "cursor-pointer hover:text-ink-2" : "cursor-default",
                              def.align === "right" && "flex-row-reverse",
                            )}
                          >
                            {Icon && <Icon className="size-3.5" />}
                            {flexRender(h.column.columnDef.header, h.getContext())}
                            {canSort &&
                              (sorted === "asc" ? (
                                <ArrowUp className="size-3 text-ink-2" />
                              ) : sorted === "desc" ? (
                                <ArrowDown className="size-3 text-ink-2" />
                              ) : (
                                <ChevronsUpDown className="size-3 opacity-40" />
                              ))}
                          </button>
                        )}
                      </th>
                    );
                  })}
                </tr>
              ))}
            </thead>
          )}
          <tbody>
            {shown.map((row) => {
              const on = isSelected(row.id);
              return (
                <tr
                  key={row.id}
                  aria-selected={selection ? on : undefined}
                  onClick={clickable ? () => onRow(row) : undefined}
                  className={cn(
                    "border-b border-line-soft last:border-b-0",
                    clickable && "cursor-pointer hover:bg-panel-2",
                    on && "bg-brand-soft hover:bg-brand-soft",
                    rowClassName?.(row.original),
                  )}
                >
                  {multi && (
                    <td className="w-9 px-3 py-2" onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        aria-label="Select row"
                        checked={on}
                        onChange={() => multi.onChange(toggleId(multi.selected, row.id))}
                        className="size-3.5 accent-brand"
                      />
                    </td>
                  )}
                  {row.getAllCells().map((cell) => {
                    const def = cell.column.columnDef as DataColumn<T>;
                    return (
                      <td
                        key={cell.id}
                        className={cn(
                          "px-3 py-2 align-middle",
                          ALIGN[def.align ?? "left"],
                          def.mono && "font-mono text-[13px]",
                          def.cellClassName,
                        )}
                      >
                        {flexRender(cell.column.columnDef.cell, cell.getContext())}
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <TableFooter
        total={rows.length}
        win={win}
        onExpand={() => setExpanded(true)}
        onCollapse={() => {
          setExpanded(false);
          setPage(1);
        }}
        onPage={setPage}
        server={serverPagination}
      />
    </div>
  );

  return (
    <QueryState
      isLoading={isLoading && data.length === 0}
      isError={isError}
      isEmpty={data.length === 0 && (serverPagination?.total ?? 0) === 0}
      empty={empty}
      onRetry={onRetry}
      className="rounded-md border border-line"
    >
      {body}
    </QueryState>
  );
}

function TableFooter({
  total,
  win,
  onExpand,
  onCollapse,
  onPage,
  server,
}: {
  total: number;
  win: ReturnType<typeof pageWindow>;
  onExpand: () => void;
  onCollapse: () => void;
  onPage: (page: number) => void;
  server?: ServerPagination;
}) {
  const serverPages = server ? Math.max(1, Math.ceil(server.total / server.pageSize)) : 1;
  const clientPager = !server && win.pageCount > 1;
  const serverPager = server != null && serverPages > 1;
  if (!win.canExpand && !win.canCollapse && !clientPager && !serverPager) return null;

  let range: string;
  if (serverPager && server) {
    const from = (server.page - 1) * server.pageSize + 1;
    range = `${from}–${from + total - 1} of ${server.total}`;
  } else {
    range = `${total === 0 ? 0 : win.start + 1}–${win.end} of ${total}`;
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line bg-card px-3 py-1.5 text-xs text-ink-3">
      <span className="font-mono">{range}</span>
      <div className="flex items-center gap-1">
        {win.canExpand && (
          <Button size="sm" variant="ghost" onClick={onExpand}>
            Show all {total}
          </Button>
        )}
        {win.canCollapse && (
          <Button size="sm" variant="ghost" onClick={onCollapse}>
            Show fewer
          </Button>
        )}
        {clientPager && (
          <Pager page={win.page} pageCount={win.pageCount} onPage={onPage} />
        )}
        {serverPager && server && (
          <Pager page={server.page} pageCount={serverPages} onPage={server.onPageChange} />
        )}
      </div>
    </div>
  );
}

function Pager({ page, pageCount, onPage }: { page: number; pageCount: number; onPage: (page: number) => void }) {
  return (
    <span className="flex items-center gap-1">
      <Button size="icon" variant="ghost" className="size-7" aria-label="Previous page" disabled={page <= 1} onClick={() => onPage(page - 1)}>
        <ChevronLeft />
      </Button>
      <span className="font-mono">
        {page}/{pageCount}
      </span>
      <Button size="icon" variant="ghost" className="size-7" aria-label="Next page" disabled={page >= pageCount} onClick={() => onPage(page + 1)}>
        <ChevronRight />
      </Button>
    </span>
  );
}

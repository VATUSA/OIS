import {useMemo} from "react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ois/ui";
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import {ArrowDown, ArrowUp, Check, ChevronsUpDown, Columns3} from "lucide-react";

import {hhmmZulu} from "@/lib/time";

import {type DataSource, DATA_SOURCES_BY_ID, type FieldType, type Row} from "./sources";
import type {TableWidget as TableWidgetT} from "./types";

function Cell({ value, type }: { value: unknown; type: FieldType }) {
  if (value == null || value === "") return <span className="text-muted-foreground">—</span>;
  if (type === "time") return <>{hhmmZulu(String(value))}</>;
  if (type === "bool") return <>{value ? "yes" : "no"}</>;
  return <>{String(value)}</>;
}

function ColumnPicker({
  source,
  visible,
  onChange,
}: {
  source: DataSource;
  visible: string[];
  onChange: (columns: string[]) => void;
}) {
  const shown = new Set(visible);
  const toggle = (key: string) => {
    // Rebuild in field order so column order stays stable; keep at least one column.
    const next = source.fields
      .map((fd) => fd.key)
      .filter((k) => (k === key ? !shown.has(k) : shown.has(k)));
    if (next.length > 0) onChange(next);
  };
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="secondary" className="h-7 self-end">
          <Columns3 />
          Columns
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-[50vh] w-48 overflow-y-auto">
        {source.fields.map((fd) => (
          <DropdownMenuItem
            key={fd.key}
            onSelect={(e) => {
              e.preventDefault();
              toggle(fd.key);
            }}
          >
            <Check className={"size-3.5 " + (shown.has(fd.key) ? "opacity-100" : "opacity-0")} />
            {fd.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Notice({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-center text-sm text-muted-foreground">{children}</p>;
}

function TableInner({
  source,
  widget,
  editing,
  onChange,
}: {
  source: DataSource;
  widget: TableWidgetT;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const { rows, isLoading, isError } = source.useRows(widget.params ?? {});
  const visible = widget.columns ?? source.fields.map((fd) => fd.key);
  const typeByKey = useMemo(
    () => Object.fromEntries(source.fields.map((fd) => [fd.key, fd.type])) as Record<string, FieldType>,
    [source],
  );

  const columns = useMemo<ColumnDef<Row>[]>(
    () =>
      visible
        .map((key) => source.fields.find((fd) => fd.key === key))
        .filter((fd): fd is NonNullable<typeof fd> => !!fd)
        .map((fd) => ({
          accessorKey: fd.key,
          header: fd.label,
          cell: (info) => <Cell value={info.getValue()} type={fd.type} />,
        })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [source, visible.join(",")],
  );

  const sorting = (widget.sort ?? []) as SortingState;
  const table = useReactTable({
    data: rows,
    columns,
    state: { sorting },
    onSortingChange: (updater) => {
      const next = typeof updater === "function" ? updater(sorting) : updater;
      onChange(widget.id, { sort: next });
    },
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <div className="flex h-full flex-col gap-2">
      {editing && (
        <ColumnPicker
          source={source}
          visible={visible}
          onChange={(cols) => onChange(widget.id, { columns: cols })}
        />
      )}
      {isError ? (
        <Notice>Couldn&apos;t load data.</Notice>
      ) : isLoading && rows.length === 0 ? (
        <Notice>Loading…</Notice>
      ) : rows.length === 0 ? (
        <Notice>No rows.</Notice>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 z-10 bg-card">
              {table.getHeaderGroups().map((hg) => (
                <tr key={hg.id} className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  {hg.headers.map((h) => {
                    const sorted = h.column.getIsSorted();
                    return (
                      <th
                        key={h.id}
                        className="cursor-pointer select-none whitespace-nowrap pb-2 pr-3 font-medium"
                        onClick={h.column.getToggleSortingHandler()}
                      >
                        <span className="inline-flex items-center gap-1">
                          {flexRender(h.column.columnDef.header, h.getContext())}
                          {sorted === "asc" ? (
                            <ArrowUp className="size-3" />
                          ) : sorted === "desc" ? (
                            <ArrowDown className="size-3" />
                          ) : (
                            <ChevronsUpDown className="size-3 opacity-30" />
                          )}
                        </span>
                      </th>
                    );
                  })}
                </tr>
              ))}
            </thead>
            <tbody>
              {table.getRowModel().rows.map((r) => (
                <tr key={r.id} className="border-t">
                  {r.getVisibleCells().map((c) => {
                    const type = typeByKey[c.column.id];
                    return (
                      <td
                        key={c.id}
                        className={
                          "py-1.5 pr-3 " +
                          (type === "number"
                            ? "text-right tabular-nums"
                            : type === "time"
                              ? "font-mono text-xs"
                              : "")
                        }
                      >
                        {flexRender(c.column.columnDef.cell, c.getContext())}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

export function TableWidget({
  widget,
  editing,
  onChange,
}: {
  widget: TableWidgetT;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const source = DATA_SOURCES_BY_ID[widget.source];
  if (!source) {
    return <div className="p-4 text-sm text-muted-foreground">Unknown data source.</div>;
  }
  // Key by source id so switching source remounts and hook order stays consistent.
  return <TableInner key={source.id} source={source} widget={widget} editing={editing} onChange={onChange} />;
}

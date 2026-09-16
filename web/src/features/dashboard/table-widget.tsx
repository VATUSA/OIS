import {useMemo} from "react";
import {
  Button,
  type DataColumn,
  DataTable,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  type SortingState,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ois/ui";
import {Check, Columns3} from "lucide-react";

import {hhmmZulu} from "@/lib/time";

import {facilityAirports, useFacilityDirectory} from "@/lib/facilities";
import {type DataSource, DATA_SOURCES_BY_ID, type FieldType, type Row} from "./sources";
import type {TableWidget as TableWidgetT} from "./types";
import {useReportWidgetStatus} from "./widget-status";

/** Shared stable reference for the unsorted state (see the note where it's used). */
const EMPTY_SORTING: SortingState = [];
/** The widget scrolls inside its grid cell, so every row renders (no cap / pages). */
const ALL_ROWS = Number.MAX_SAFE_INTEGER;

function Cell({ value, type }: { value: unknown; type: FieldType }) {
  if (value == null || value === "") return <span className="text-ink-3">—</span>;
  if (type === "time") return <>{hhmmZulu(String(value))}</>;
  if (type === "bool") return <>{value ? "yes" : "no"}</>;
  // Free-text fields can run long — cap the width and reveal the full value on hover rather than
  // wrapping the row or overflowing the column.
  if (type === "string") {
    const text = String(value);
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="block max-w-[16rem] truncate">{text}</span>
        </TooltipTrigger>
        <TooltipContent>{text}</TooltipContent>
      </Tooltip>
    );
  }
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
        <Button size="sm" variant="outline" className="h-7 self-end">
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
  // A facility scope expands to its member airports at render (membership stays current).
  const dir = useFacilityDirectory();
  const params = widget.params?.facility
    ? { ...widget.params, icaos: facilityAirports(dir.data, widget.params.facility.id) }
    : (widget.params ?? {});
  const { rows, isLoading, isError, isFetching, dataUpdatedAt, refetch } = source.useRows(params);
  useReportWidgetStatus(isFetching, dataUpdatedAt, refetch);
  const visible = widget.columns ?? source.fields.map((fd) => fd.key);

  const columns = useMemo<DataColumn<Row>[]>(
    () =>
      visible
        .map((key) => source.fields.find((fd) => fd.key === key))
        .filter((fd): fd is NonNullable<typeof fd> => !!fd)
        .map((fd) => ({
          accessorKey: fd.key,
          header: fd.label,
          mono: fd.type === "time" || fd.type === "number",
          align: fd.type === "number" ? ("right" as const) : undefined,
          cell: (info) => <Cell value={info.getValue()} type={fd.type} />,
        })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [source, visible.join(",")],
  );

  // Stable empty reference when unsorted — a fresh `[]` each render would churn the table state.
  const sorting = (widget.sort ?? EMPTY_SORTING) as SortingState;

  return (
    <div className="flex h-full flex-col gap-2">
      {editing && (
        <ColumnPicker
          source={source}
          visible={visible}
          onChange={(cols) => onChange(widget.id, { columns: cols })}
        />
      )}
      <div className="min-h-0 flex-1">
        <DataTable
          label={source.label}
          columns={columns}
          data={rows}
          sort={sorting}
          onSortChange={(next) => onChange(widget.id, { sort: next })}
          stickyHeader
          rowCap={ALL_ROWS}
          pageSize={ALL_ROWS}
          isLoading={isLoading}
          isError={isError}
          onRetry={refetch}
          empty="No rows."
          // A bounded flex column lets the table's own scroll area hold the sticky header.
          className="flex max-h-full flex-col"
        />
      </div>
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
    return <div className="p-4 text-sm text-ink-3">Unknown data source.</div>;
  }
  // Key by source id so switching source remounts and hook order stays consistent.
  return <TableInner key={source.id} source={source} widget={widget} editing={editing} onChange={onChange} />;
}

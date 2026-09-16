import {useMemo, useState} from "react";
import {type DataColumn, DataTable, SegmentedControl, Sheet, StatusPill, toneText, type Tone} from "@ois/ui";
import {ChevronDown, ChevronRight} from "lucide-react";

import {type Fca, type FcaFlight} from "@/lib/fca";
import {FLIGHT_STATE_LABEL, toneOf} from "@/lib/status";
import {hhmmZulu} from "@/lib/time";

type Filter = "all" | "air" | "cfr";

const FILTERS = [
  { value: "all" as const, label: "All" },
  { value: "air" as const, label: "Air" },
  { value: "cfr" as const, label: "CFR" },
];

/** A crossing flight's state tone and label (unknown states read as ground). */
function statusOf(s: string): { tone: Tone; label: string } {
  const state = toneOf("flight", s) !== "neutral" ? s : "ground";
  return { tone: toneOf("flight", state), label: FLIGHT_STATE_LABEL[state] };
}

const modeLabel = (fca: Fca) => (fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`);

function matchesFilter(f: FcaFlight, filter: Filter): boolean {
  if (filter === "air") return f.status === "airborne";
  if (filter === "cfr") return f.released;
  return true;
}

export interface OverviewGroup {
  fca: Fca;
  flights: FcaFlight[] | undefined;
}

/** One read-only crossing strip per row: sequence, state, flight, crossing time + delay. */
const STRIP_COLUMNS: DataColumn<FcaFlight>[] = [
  {
    accessorKey: "seq",
    header: "#",
    mono: true,
    align: "right",
    cellClassName: "w-8 px-2 text-xs text-ink-3",
  },
  {
    accessorKey: "callsign",
    header: "Flight",
    cellClassName: "px-2",
    cell: (c) => {
      const f = c.row.original;
      const st = statusOf(f.status);
      return (
        <span className="flex min-w-0 items-center gap-2 text-xs">
          <StatusPill tone={st.tone} className="px-1.5 text-[10px] leading-4">
            {f.released ? "CFR" : st.label}
          </StatusPill>
          <span className="font-mono font-semibold">{f.callsign}</span>
          <span className="truncate font-mono text-ink-3">
            {f.dep}→{f.arr}
          </span>
        </span>
      );
    },
  },
  {
    accessorKey: "cross_time",
    header: "Crossing",
    mono: true,
    align: "right",
    cellClassName: "px-2 text-xs leading-tight",
    cell: (c) => {
      const f = c.row.original;
      const delayed = f.delay_sec >= 30;
      return (
        <span className="whitespace-nowrap">
          <span className={delayed ? "text-ink" : toneText[statusOf(f.status).tone]}>{hhmmZulu(f.cross_time)}</span>
          <span className={`block text-[10px] ${delayed ? "text-danger" : "text-success"}`}>
            {delayed ? `+${Math.round(f.delay_sec / 60)}m` : "on time"}
          </span>
        </span>
      );
    },
  },
];

/**
 * The ARTCC overview's right-hand "strips" panel: every active FCA in the selected ARTCC stacked as a
 * compact, collapsible card showing its crossing traffic. Read-only — this is the public advisories
 * view; controllers manage releases from the per-FCA tool. Mirrors vatflow's artcc-dashboard strips.
 */
export function FcaOverviewPanel({
  artcc,
  groups,
  onFocusFlight,
  onClose,
}: {
  artcc: string;
  groups: OverviewGroup[];
  onFocusFlight?: (callsign: string) => void;
  onClose?: () => void;
}) {
  const [filter, setFilter] = useState<Filter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <Sheet className="h-full w-96 shrink-0 border-l border-line" onClose={onClose} initialFraction={0.5}>
      <div className="flex items-center gap-2 border-b border-line px-4 py-3">
        <span className="text-sm font-semibold">
          Strips · <span className="font-mono">{artcc}</span>
        </span>
        <SegmentedControl
          aria-label="Filter strips"
          size="sm"
          className="ml-auto"
          value={filter}
          onChange={setFilter}
          options={FILTERS}
        />
        {/* Clear the mobile sheet's close X. */}
        <span className="w-6 shrink-0 md:hidden" />
      </div>

      <div className="flex-1 overflow-y-auto">
        {groups.length === 0 ? (
          <p className="p-4 text-sm text-ink-2">No active FCAs for {artcc}.</p>
        ) : (
          groups.map((g) => (
            <OverviewGroupSection
              key={g.fca.id}
              group={g}
              filter={filter}
              collapsed={collapsed.has(g.fca.id)}
              onToggle={() => toggle(g.fca.id)}
              onFocusFlight={onFocusFlight}
            />
          ))
        )}
      </div>
    </Sheet>
  );
}

function OverviewGroupSection({
  group: { fca, flights },
  filter,
  collapsed,
  onToggle,
  onFocusFlight,
}: {
  group: OverviewGroup;
  filter: Filter;
  collapsed: boolean;
  onToggle: () => void;
  onFocusFlight?: (callsign: string) => void;
}) {
  const list = useMemo(() => (flights ?? []).filter((f) => f.status !== "proposed"), [flights]);
  const shown = useMemo(() => list.filter((f) => matchesFilter(f, filter)), [list, filter]);
  const air = list.filter((f) => f.status === "airborne").length;
  const cfr = list.filter((f) => f.released).length;

  return (
    <div className="border-b border-line">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-panel-2"
      >
        {collapsed ? (
          <ChevronRight className="size-3.5 shrink-0 text-ink-3" />
        ) : (
          <ChevronDown className="size-3.5 shrink-0 text-ink-3" />
        )}
        {/* The FCA's colour is user data. */}
        <span className="size-2.5 shrink-0 rounded-full" style={{ background: fca.color }} />
        <span className="truncate font-mono text-sm font-semibold">{fca.name}</span>
        <StatusPill tone="neutral" className="font-mono">
          {modeLabel(fca)}
        </StatusPill>
        {fca.manual_seq && <StatusPill tone="neutral">Manual</StatusPill>}
        <span className="ml-auto shrink-0 font-mono text-xs text-ink-3">
          {air} air · {cfr} CFR
        </span>
      </button>
      {!collapsed && (
        <div className="px-3 pb-3">
          <DataTable
            label={`${fca.name} crossing traffic`}
            columns={STRIP_COLUMNS}
            data={shown}
            getRowId={(f) => f.callsign}
            hideHeader
            rowCap={25}
            onRowClick={(f) => onFocusFlight?.(f.callsign)}
            isLoading={flights == null}
            empty={`No ${filter === "air" ? "airborne " : filter === "cfr" ? "CFR " : ""}traffic crossing this FCA.`}
          />
        </div>
      )}
    </div>
  );
}

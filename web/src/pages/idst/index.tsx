import {useMemo, useState} from "react";
import {useQueryClient} from "@tanstack/react-query";
import {
  Button,
  Card,
  type DataColumn,
  DataTable,
  EmptyState,
  FilterBar,
  FilterChip,
  Input,
  MetricCard,
  Select,
  StatusPill,
} from "@ois/ui";
import {Clock, Plane, RefreshCw, Route, Navigation} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useClearRelease, useMarkRelease, useSwapReleases} from "@/lib/fca";
import {swapPartners, useIdst, useIdstScope, scopeIsEmpty, type IdstFlight, type IdstScope} from "@/lib/idst";
import {FacilityCombobox, type FacilityPick} from "@/components/facility-combobox";

const keyOf = (f: IdstFlight) => `${f.fca_id}:${f.callsign}`;

/** "1236z" from an ISO time (or an em dash). */
function hhmmZ(iso?: string | null): string {
  if (!iso) return "—";
  return `${new Date(iso).toISOString().slice(11, 16).replace(":", "")}z`;
}

function ScopePanel({ scope, setScope }: { scope: IdstScope; setScope: (s: IdstScope) => void }) {
  const [airport, setAirport] = useState("");
  const addAirport = () => {
    const a = airport.trim().toUpperCase();
    if (/^[A-Z0-9]{3,4}$/.test(a) && !scope.airports.includes(a)) {
      setScope({ ...scope, airports: [...scope.airports, a] });
    }
    setAirport("");
  };
  const addFacility = (pick: FacilityPick) => {
    const list = pick.kind === "artcc" ? "artccs" : "tracons";
    if (!scope[list].includes(pick.id)) setScope({ ...scope, [list]: [...scope[list], pick.id] });
  };
  const remove = (list: keyof IdstScope, v: string) =>
    setScope({ ...scope, [list]: scope[list].filter((x) => x !== v) });

  return (
    <Card className="flex flex-col gap-4 p-4">
      <h2 className="text-xl font-bold">Working</h2>

      <div className="flex flex-col gap-1.5">
        <label htmlFor="idst-airport" className="text-xs font-semibold text-ink-2">
          Tower · Airport
        </label>
        <div className="flex gap-1.5">
          <Input
            id="idst-airport"
            value={airport}
            onChange={(e) => setAirport(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addAirport()}
            placeholder="KDCA"
            className="h-8 font-mono uppercase"
          />
          <Button size="sm" variant="outline" onClick={addAirport}>
            Add
          </Button>
        </div>
        {scope.airports.length > 0 && (
          <FilterBar className="gap-1.5">
            {scope.airports.map((a) => (
              <FilterChip key={a} label="Airport" value={a} active onClear={() => remove("airports", a)} />
            ))}
          </FilterBar>
        )}
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-semibold text-ink-2">Facility · TRACON / ARTCC</span>
        <FacilityCombobox
          onSelect={addFacility}
          placeholder="PCT, ZDC, N90…"
          className="w-full"
          inputClassName="h-8"
        />
        {scope.tracons.length + scope.artccs.length > 0 && (
          <FilterBar className="gap-1.5">
            {scope.tracons.map((t) => (
              <FilterChip key={t} label="TRACON" value={t} active onClear={() => remove("tracons", t)} />
            ))}
            {scope.artccs.map((z) => (
              <FilterChip key={z} label="ARTCC" value={z} active onClear={() => remove("artccs", z)} />
            ))}
          </FilterBar>
        )}
      </div>

      <p className="mt-auto text-xs text-ink-3">
        Your scope syncs to your account. Only FCA-metered ground departures in scope appear in Flights to Work.
      </p>
    </Card>
  );
}

const COLUMNS: DataColumn<IdstFlight>[] = [
  {
    accessorKey: "callsign",
    header: "Callsign",
    icon: Plane,
    mono: true,
    cellClassName: "font-semibold",
  },
  {
    id: "route",
    accessorFn: (f) => `${f.dep} ${f.arr}`,
    header: "Route",
    icon: Route,
    cell: (c) => {
      const f = c.row.original;
      return (
        <span className="whitespace-nowrap font-mono text-xs">
          {f.dep} → {f.arr}
          <span className="ml-1.5 text-ink-3">{f.aircraft_type || "—"}</span>
        </span>
      );
    },
  },
  {
    accessorKey: "fca_name",
    header: "FCA",
    cell: (c) => <StatusPill tone="neutral">{c.row.original.fca_name}</StatusPill>,
  },
  {
    id: "runway",
    accessorFn: (f) => f.runway ?? "",
    header: "Rwy",
    icon: Navigation,
    mono: true,
    cell: (c) => {
      const f = c.row.original;
      // No prediction is a real answer, not a gap: nothing could derive one (no airport configuration,
      // or no rule and no configured default). Showing a dash rather than blank says so.
      if (!f.runway) return <span className="text-ink-3">—</span>;
      return (
        <span className="whitespace-nowrap font-mono text-xs">
          {f.runway}
          {/* The rung that chose it, so a controller can see *why* — and in particular whether a human
              set it by hand (#511 AC1). */}
          {f.runway_source ? <span className="ml-1.5 text-ink-3">{f.runway_source}</span> : null}
        </span>
      );
    },
  },
  {
    id: "edct",
    accessorFn: (f) => (f.edct ? new Date(f.edct).getTime() : Infinity),
    header: "EDCT",
    icon: Clock,
    mono: true,
    align: "right",
    sortDescFirst: false,
    cell: (c) => {
      const f = c.row.original;
      return f.released ? (
        <span className="whitespace-nowrap text-success">
          RLSD {hhmmZ(f.edct)}
          {/* A time an external tool issued says so, so it is never unexplained (#585). */}
          {f.released_by_machine ? (
            <span data-release-source className="text-ink-3"> · via {f.released_by_machine}</span>
          ) : null}
        </span>
      ) : (
        <span className="whitespace-nowrap text-ink-2">
          EDCT {hhmmZ(f.edct)}
          {f.delay_min > 0 ? ` · +${f.delay_min}m` : ""}
        </span>
      );
    },
  },
];

export function FlightTable({
  title,
  flights,
  selKey,
  onSelect,
  empty,
}: {
  title: string;
  flights: IdstFlight[];
  selKey: string | null;
  onSelect: (k: string | null) => void;
  empty: string;
}) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="flex items-baseline gap-2 text-xl font-bold">
        {title}
        <span className="font-mono text-sm font-normal text-ink-3">{flights.length}</span>
      </h2>
      <DataTable
        label={title}
        columns={COLUMNS}
        data={flights}
        getRowId={keyOf}
        // Live ops list: always pages, never hides rows behind "Show all".
        rowCap={Infinity}
        pageSize={25}
        // Clicking the working flight again keeps it selected (the CFR panel stays open), as before.
        selection={{ mode: "single", selected: selKey, onChange: (k) => k && onSelect(k) }}
        empty={empty}
      />
    </section>
  );
}

/** Trade the selected release's times with another on its runway. */
function SwapControl({ selected, released }: { selected: IdstFlight; released: IdstFlight[] }) {
  const partners = swapPartners(selected, released);
  const [partner, setPartner] = useState("");
  const swap = useSwapReleases(selected.fca_id);
  if (partners.length === 0) {
    return (
      <p className="text-xs text-ink-3">
        {selected.runway
          ? `No other release departs ${selected.dep} runway ${selected.runway} in this FCA to swap with.`
          : "No departure runway is assigned, so this release can't be swapped."}
      </p>
    );
  }
  const chosen = partners.some((f) => f.callsign === partner) ? partner : "";
  return (
    <div className="flex gap-1.5">
      <Select
        aria-label="Swap times with"
        value={chosen}
        onChange={(e) => setPartner(e.target.value)}
        wrapperClassName="flex-1"
        className="font-mono"
      >
        <option value="">Swap with…</option>
        {partners.map((f) => (
          <option key={f.callsign} value={f.callsign}>
            {f.callsign} · {hhmmZ(f.edct)}
          </option>
        ))}
      </Select>
      <Button
        variant="outline"
        disabled={!chosen || swap.isPending}
        onClick={() => swap.mutate({ a: selected.callsign, b: chosen }, { onSuccess: () => setPartner("") })}
      >
        Swap
      </Button>
    </div>
  );
}

function SelectedPanel({
  selected,
  released,
  canEdit,
}: {
  selected: IdstFlight | null;
  released: IdstFlight[];
  canEdit: boolean;
}) {
  const qc = useQueryClient();
  const [ready, setReady] = useState("");
  const mark = useMarkRelease(selected?.fca_id ?? "");
  const clear = useClearRelease(selected?.fca_id ?? "");
  const invalidate = () => qc.invalidateQueries({ queryKey: ["idst"] });

  return (
    <Card className="flex flex-col gap-3 p-4">
      <h2 className="text-xl font-bold">Selected flight</h2>
      {!selected ? (
        <p className="text-sm text-ink-2">Select a flight to issue or cancel a CFR release.</p>
      ) : (
        <>
          <div>
            <div className="font-mono text-lg font-bold">{selected.callsign}</div>
            <div className="font-mono text-sm text-ink-2">
              {selected.dep} → {selected.arr} · {selected.aircraft_type || "—"}
            </div>
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs">
              <StatusPill tone="neutral">{selected.fca_name}</StatusPill>
              <span className="font-mono text-ink-3">seq {selected.seq}</span>
              {selected.delay_min > 0 && (
                <span className="font-mono text-warning">+{selected.delay_min}m delay</span>
              )}
            </div>
          </div>

          <dl className="divide-y divide-line-soft border-y border-line-soft text-sm">
            <div className="flex justify-between py-1.5">
              <dt className="text-ink-2">CTA (crossing)</dt>
              <dd className="font-mono">{hhmmZ(selected.cross_time)}</dd>
            </div>
            <div className="flex justify-between py-1.5">
              <dt className="text-ink-2">{selected.released ? "EDCT (wheels-up)" : "Proposed EDCT"}</dt>
              <dd className={`font-mono ${selected.released ? "text-success" : ""}`}>{hhmmZ(selected.edct)}</dd>
            </div>
            {selected.released_by_machine ? (
              <div className="flex justify-between py-1.5">
                <dt className="text-ink-2">Released by</dt>
                <dd className="font-mono">{selected.released_by_machine}</dd>
              </div>
            ) : null}
          </dl>

          {canEdit ? (
            selected.released ? (
              <div className="flex flex-col gap-2">
                <SwapControl key={keyOf(selected)} selected={selected} released={released} />
                <Button
                  variant="destructive"
                  onClick={() => clear.mutate(selected.callsign, { onSuccess: invalidate })}
                  disabled={clear.isPending}
                >
                  Cancel release
                </Button>
              </div>
            ) : (
              <div className="flex flex-col gap-2">
                <div className="flex gap-1.5">
                  <Input
                    aria-label="Ready time"
                    value={ready}
                    onChange={(e) => setReady(e.target.value)}
                    placeholder="HHMMz"
                    maxLength={5}
                    className="h-9 font-mono"
                  />
                  <Button
                    variant="outline"
                    disabled={ready.trim().length < 4 || mark.isPending}
                    onClick={() => mark.mutate({ callsign: selected.callsign, ready: ready.trim() }, { onSuccess: () => { setReady(""); invalidate(); } })}
                  >
                    Set
                  </Button>
                </div>
                <Button
                  disabled={mark.isPending}
                  onClick={() => mark.mutate({ callsign: selected.callsign }, { onSuccess: invalidate })}
                >
                  RDY — release earliest
                </Button>
              </div>
            )
          ) : (
            <p className="text-xs text-ink-3">You don&apos;t have permission to issue releases.</p>
          )}
        </>
      )}
    </Card>
  );
}

/** IDST — the Integrated Departure Scheduling console for FCA release timing. */
export function IdstPage() {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "flow.fca.update");
  const { scope, setScope } = useIdstScope();
  const idst = useIdst(scope);
  const [selKey, setSelKey] = useState<string | null>(null);

  const unscheduled = idst.data?.unscheduled ?? [];
  const released = idst.data?.released ?? [];
  const selected = [...unscheduled, ...released].find((f) => keyOf(f) === selKey) ?? null;

  const { refetch, isFetching } = idst;
  const actions = useMemo(
    () => (
      <Button size="sm" variant="outline" onClick={() => refetch()}>
        <RefreshCw className={isFetching ? "animate-spin" : undefined} />
        Refresh
      </Button>
    ),
    [refetch, isFetching],
  );
  usePageHeader({ subtitle: "Integrated Departure Scheduling — FCA release timing.", actions });

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-3 gap-3">
        <MetricCard label="Unscheduled" value={unscheduled.length} />
        <MetricCard label="Released" value={released.length} tone={released.length > 0 ? "good" : undefined} />
        <MetricCard label="Metered" value={idst.data?.metered_count ?? 0} />
      </div>

      <div className="grid items-start gap-4 lg:grid-cols-[280px_minmax(0,1fr)_320px]">
        <ScopePanel scope={scope} setScope={setScope} />
        {scopeIsEmpty(scope) ? (
          <EmptyState className="min-h-[16rem] rounded-md border border-line">
            Set your scope to see FCA-metered ground departures.
          </EmptyState>
        ) : (
          <div className="flex min-w-0 flex-col gap-6">
            <FlightTable
              title="Unscheduled"
              flights={unscheduled}
              selKey={selKey}
              onSelect={setSelKey}
              empty="No unscheduled metered departures in scope"
            />
            <FlightTable
              title="Released"
              flights={released}
              selKey={selKey}
              onSelect={setSelKey}
              empty="No frozen CFR releases in scope"
            />
          </div>
        )}
        <SelectedPanel selected={selected} released={released} canEdit={canEdit} />
      </div>
    </div>
  );
}

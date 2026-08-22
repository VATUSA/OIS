import {useState} from "react";
import {useQueryClient} from "@tanstack/react-query";
import {Badge, Button, Input} from "@ois/ui";
import {RefreshCw, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useClearRelease, useMarkRelease} from "@/lib/fca";
import {useIdst, useIdstScope, scopeIsEmpty, type IdstFlight, type IdstScope} from "@/lib/idst";
import {FacilityCombobox, type FacilityPick} from "@/components/facility-combobox";

const keyOf = (f: IdstFlight) => `${f.fca_id}:${f.callsign}`;

/** "1236z" from an ISO time (or an em dash). */
function hhmmZ(iso?: string | null): string {
  if (!iso) return "—";
  return `${new Date(iso).toISOString().slice(11, 16).replace(":", "")}z`;
}

/** A removable scope chip. */
function Chip({ label, color, onRemove }: { label: string; color: string; onRemove: () => void }) {
  return (
    <span
      className={`flex items-center gap-1 rounded border px-2 py-0.5 text-xs font-medium ${color}`}
    >
      {label}
      <button type="button" onClick={onRemove} aria-label={`Remove ${label}`}>
        <X className="size-3 opacity-70 hover:opacity-100" />
      </button>
    </span>
  );
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
    <aside className="flex flex-col gap-4 rounded-lg border p-3">
      <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Working</div>

      <div className="flex flex-col gap-1.5">
        <label className="text-xs font-medium text-muted-foreground">Tower · Airport</label>
        <div className="flex gap-1.5">
          <Input
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
        <div className="flex flex-wrap gap-1.5">
          {scope.airports.map((a) => (
            <Chip key={a} label={a} color="border-sky-500/40 text-sky-600 dark:text-sky-400" onRemove={() => remove("airports", a)} />
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <label className="text-xs font-medium text-muted-foreground">Facility · TRACON / ARTCC</label>
        <FacilityCombobox onSelect={addFacility} placeholder="PCT, ZDC, N90…" />
        <div className="flex flex-wrap gap-1.5">
          {scope.tracons.map((t) => (
            <Chip key={t} label={t} color="border-emerald-500/40 text-emerald-600 dark:text-emerald-400" onRemove={() => remove("tracons", t)} />
          ))}
          {scope.artccs.map((z) => (
            <Chip key={z} label={z} color="border-violet-500/40 text-violet-600 dark:text-violet-400" onRemove={() => remove("artccs", z)} />
          ))}
        </div>
      </div>

      <p className="mt-auto text-xs text-muted-foreground">
        Your scope syncs to your account. Only FCA-metered ground departures in scope appear in Flights to Work.
      </p>
    </aside>
  );
}

function FlightRow({
  f,
  selected,
  onClick,
}: {
  f: IdstFlight;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`grid w-full grid-cols-[1fr_auto] items-center gap-2 border-b px-3 py-2 text-left text-sm last:border-0 hover:bg-accent ${
        selected ? "bg-accent" : ""
      }`}
    >
      <span className="min-w-0">
        <span className="font-mono font-medium">{f.callsign}</span>
        <span className="ml-2 text-xs text-muted-foreground">
          {f.dep} → {f.arr} · {f.aircraft_type || "—"}
        </span>
        <span className="ml-2">
          <Badge variant="secondary" className="text-[10px]">{f.fca_name}</Badge>
        </span>
      </span>
      <span className="whitespace-nowrap font-mono text-xs">
        {f.released ? (
          <span className="text-emerald-600 dark:text-emerald-400">RLSD {hhmmZ(f.edct)}</span>
        ) : (
          <span className="text-muted-foreground">
            EDCT {hhmmZ(f.edct)}
            {f.delay_min > 0 ? ` · +${f.delay_min}m` : ""}
          </span>
        )}
      </span>
    </button>
  );
}

function Column({
  title,
  count,
  flights,
  selKey,
  onSelect,
  empty,
  accent,
}: {
  title: string;
  count: number;
  flights: IdstFlight[];
  selKey: string | null;
  onSelect: (k: string) => void;
  empty: string;
  accent: string;
}) {
  return (
    <div className="flex min-h-[16rem] flex-col rounded-lg border">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <span className={`text-sm font-semibold ${accent}`}>{title}</span>
        <span className="text-xs text-muted-foreground">{count}</span>
      </div>
      {flights.length === 0 ? (
        <p className="flex flex-1 items-center justify-center p-6 text-center text-sm text-muted-foreground">
          {empty}
        </p>
      ) : (
        <div className="flex-1 overflow-y-auto">
          {flights.map((f) => (
            <FlightRow key={keyOf(f)} f={f} selected={selKey === keyOf(f)} onClick={() => onSelect(keyOf(f))} />
          ))}
        </div>
      )}
    </div>
  );
}

function SelectedPanel({ selected, canEdit }: { selected: IdstFlight | null; canEdit: boolean }) {
  const qc = useQueryClient();
  const [ready, setReady] = useState("");
  const mark = useMarkRelease(selected?.fca_id ?? "");
  const clear = useClearRelease(selected?.fca_id ?? "");
  const invalidate = () => qc.invalidateQueries({ queryKey: ["idst"] });

  return (
    <aside className="flex flex-col gap-3 rounded-lg border p-3">
      <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Selected flight</div>
      {!selected ? (
        <p className="text-sm text-muted-foreground">Select a flight to issue or cancel a CFR release.</p>
      ) : (
        <>
          <div>
            <div className="font-mono text-lg font-semibold">{selected.callsign}</div>
            <div className="text-sm text-muted-foreground">
              {selected.dep} → {selected.arr} · {selected.aircraft_type || "—"}
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs">
              <Badge variant="secondary">{selected.fca_name}</Badge>
              <span className="text-muted-foreground">seq {selected.seq}</span>
              {selected.delay_min > 0 && <span className="text-amber-600 dark:text-amber-400">+{selected.delay_min}m delay</span>}
            </div>
          </div>

          <div className="rounded border bg-muted/30 p-2 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">CTA (crossing)</span>
              <span className="font-mono">{hhmmZ(selected.cross_time)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">
                {selected.released ? "EDCT (wheels-up)" : "Proposed EDCT"}
              </span>
              <span
                className={`font-mono ${selected.released ? "text-emerald-600 dark:text-emerald-400" : ""}`}
              >
                {hhmmZ(selected.edct)}
              </span>
            </div>
          </div>

          {canEdit ? (
            selected.released ? (
              <Button
                variant="destructive"
                onClick={() => clear.mutate(selected.callsign, { onSuccess: invalidate })}
                disabled={clear.isPending}
              >
                Cancel release
              </Button>
            ) : (
              <div className="flex flex-col gap-2">
                <div className="flex gap-1.5">
                  <Input
                    value={ready}
                    onChange={(e) => setReady(e.target.value)}
                    placeholder="HHMMz"
                    maxLength={5}
                    className="h-9 font-mono"
                  />
                  <Button
                    variant="secondary"
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
            <p className="text-xs text-muted-foreground">You don&apos;t have permission to issue releases.</p>
          )}
        </>
      )}
    </aside>
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

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">IDST</h1>
          <p className="text-muted-foreground">Integrated Departure Scheduling — FCA release timing.</p>
        </div>
        <div className="flex items-center gap-3 text-sm text-muted-foreground">
          <span>UNSCHED {unscheduled.length}</span>
          <span>RELEASED {released.length}</span>
          <span>METERED {idst.data?.metered_count ?? 0}</span>
          <button type="button" onClick={() => idst.refetch()} title="Refresh" className="hover:text-foreground">
            <RefreshCw className={`size-4 ${idst.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-[280px_1fr_320px]">
        <ScopePanel scope={scope} setScope={setScope} />
        {scopeIsEmpty(scope) ? (
          <div className="flex min-h-[16rem] items-center justify-center rounded-lg border text-sm text-muted-foreground lg:col-span-1">
            Set your scope to see FCA-metered ground departures.
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2">
            <Column
              title="Unscheduled"
              count={unscheduled.length}
              flights={unscheduled}
              selKey={selKey}
              onSelect={setSelKey}
              empty="No unscheduled metered departures in scope"
              accent=""
            />
            <Column
              title="Released"
              count={released.length}
              flights={released}
              selKey={selKey}
              onSelect={setSelKey}
              empty="No frozen CFR releases in scope"
              accent="text-emerald-600 dark:text-emerald-400"
            />
          </div>
        )}
        <SelectedPanel selected={selected} canEdit={canEdit} />
      </div>
    </div>
  );
}

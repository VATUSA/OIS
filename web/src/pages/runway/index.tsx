import {createContext, useContext, useMemo, useState} from "react";
import {
  Bars,
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  EmptyState,
  Input,
  Select,
  Sheet,
  StatusPill,
  usePrompt,
} from "@ois/ui";
import {Lock, PlaneLanding, SlidersHorizontal, X} from "lucide-react";

import {ZuluClock} from "@/components/zulu-clock";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {
  type RunwayArrival,
  type RunwayEnd,
  useDeleteConfig,
  useRunway,
  useSaveConfig,
  useSavedConfigs,
  useUpdateRunway,
} from "@/lib/runway";

const BIN_MAX = 5;
const PRESET_HDG: Record<string, number> = { W: 270, E: 90, N: 360, S: 180 };
/** Balancer load level → level token (unknown reads as ok, like the original bins). */
const LEVEL_TOKEN: Record<string, string> = {
  green: "level-ok",
  yellow: "level-watch",
  red: "level-over",
};
const PRESETS = [
  { id: "W", label: "West" },
  { id: "E", label: "East" },
  { id: "N", label: "North" },
  { id: "S", label: "South" },
  { id: "OFF", label: "None" },
];

function angleDiff(a: number, b: number): number {
  return Math.abs((((a - b) % 360) + 540) % 360 - 180);
}
function zulu(iso: string): string {
  const d = new Date(iso);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCHours())}${p(d.getUTCMinutes())}Z`;
}
function minsFromNow(iso: string): number {
  return Math.round((new Date(iso).getTime() - Date.now()) / 60000);
}
/** STAR base name — strip the trailing revision (CAMRN4 → CAMRN). */
function starBase(name: string): string {
  return name.trim().toUpperCase().replace(/\d[A-Z]?$/, "");
}

export function RunwayPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "flow.runway.read");
  const canEdit = hasPermission(me, "flow.runway.update");

  const [field, setField] = useState("");
  const [icao, setIcao] = useState<string | null>(null);
  const [newStar, setNewStar] = useState("");
  const [newRwy, setNewRwy] = useState("");
  const [newEndId, setNewEndId] = useState("");
  const [newEndHdg, setNewEndHdg] = useState("");
  const board = useRunway(icao);
  const update = useUpdateRunway(icao ?? "");
  const configs = useSavedConfigs(icao);
  const saveCfg = useSaveConfig(icao ?? "");
  const deleteCfg = useDeleteConfig(icao ?? "");
  const prompt = usePrompt();
  const [selectedCfg, setSelectedCfg] = useState("");
  // Mobile: the config panel is a bottom drawer opened from the header.
  const [configOpen, setConfigOpen] = useState(false);
  const b = board.data;

  const activeIds = useMemo(
    () => (b?.ends ?? []).filter((e) => e.active).map((e) => e.id),
    [b?.ends],
  );

  // Each edit sends only the field(s) it changes; the server coalesces the rest, so a
  // controller toggling a runway can't clobber another's STAR rules on this shared board.
  function save(active: string[]) {
    if (!icao || !b) return;
    update.mutate({ active_ends: active });
  }
  function setWindow(windowMin: number) {
    if (!icao || !b) return;
    update.mutate({ window_min: windowMin });
  }
  function toggleEnd(id: string) {
    if (!canEdit) return;
    const next = activeIds.includes(id)
      ? activeIds.filter((x) => x !== id)
      : [...activeIds, id];
    save(next);
  }
  function applyPreset(p: string) {
    if (!canEdit || !b) return;
    const next =
      p === "OFF"
        ? []
        : b.ends
            .filter((e) => angleDiff(e.hdg, PRESET_HDG[p]) <= 65)
            .map((e) => e.id);
    save(next);
  }
  function saveRules(rules: Record<string, string>) {
    if (!icao || !b) return;
    update.mutate({ star_rules: rules });
  }
  function addRule() {
    const s = starBase(newStar);
    if (!canEdit || !b || !s || !newRwy) return;
    saveRules({ ...b.star_rules, [s]: newRwy });
    setNewStar("");
    setNewRwy("");
  }
  function removeRule(star: string) {
    if (!b) return;
    const next = { ...b.star_rules };
    delete next[star];
    saveRules(next);
  }
  function applyConfig(name: string) {
    setSelectedCfg(name);
    const cfg = configs.data?.find((c) => c.name === name);
    if (!cfg || !b || !icao) return;
    update.mutate({
      active_ends: cfg.active_ends,
      star_rules: cfg.star_rules,
    });
  }
  async function saveCurrentConfig() {
    if (!b) return;
    const name = (
      await prompt({
        title: "Save runway config",
        label: "Config name",
        placeholder: "e.g. West flow",
        confirmText: "Save",
      })
    )?.trim();
    if (!name) return;
    saveCfg.mutate({ name, active_ends: activeIds, star_rules: b.star_rules });
    setSelectedCfg(name);
  }
  function deleteSelectedConfig() {
    if (!selectedCfg) return;
    deleteCfg.mutate(selectedCfg);
    setSelectedCfg("");
  }
  function addEnd() {
    if (!canEdit || !b) return;
    const id = newEndId.trim().toUpperCase();
    const hdg = parseInt(newEndHdg.trim(), 10);
    if (!id || !Number.isFinite(hdg)) return;
    const custom_ends = [
      ...(b.custom_ends ?? []).filter((c) => c.id !== id),
      { id, hdg: ((hdg % 360) + 360) % 360, len: 0 },
    ];
    update.mutate({ custom_ends });
    setNewEndId("");
    setNewEndHdg("");
  }
  function removeEnd(id: string) {
    if (!canEdit || !b) return;
    update.mutate({
      active_ends: activeIds.filter((x) => x !== id),
      custom_ends: (b.custom_ends ?? []).filter((c) => c.id !== id),
    });
  }
  function setOverride(cs: string, rwy: string) {
    if (!canEdit || !b) return;
    const next: Record<string, string> = { ...b.overrides };
    if (rwy === "AUTO") delete next[cs];
    else next[cs] = rwy;
    // Prune overrides for aircraft that are no longer arriving.
    const live = new Set((b.arrivals ?? []).map((a) => a.cs));
    const pruned = Object.fromEntries(
      Object.entries(next).filter(([c]) => live.has(c)),
    );
    update.mutate({ overrides: pruned });
  }

  const load = () => {
    const f = field.trim().toUpperCase();
    if (f) setIcao(f);
  };

  if (!canRead) {
    return (
      <EmptyState icon={Lock} className="h-full">
        You don&apos;t have flow access.
      </EmptyState>
    );
  }

  const arrivals = b?.arrivals ?? [];
  const pairs = groupPairs(b?.ends ?? []);
  const recMap = new Map((b?.recs ?? []).map((r) => [r.cs, r]));
  const customIds = new Set((b?.custom_ends ?? []).map((c) => c.id));

  return (
    <div className="flex h-full flex-col bg-panel">
      {/* Control row */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-4 py-2.5 text-sm sm:px-5">
        <Input
          aria-label="Airport"
          value={field}
          onChange={(e) => setField(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && load()}
          placeholder="ICAO"
          className="h-8 w-24 font-mono uppercase sm:w-28"
        />
        <Button size="sm" onClick={load} disabled={!field.trim()}>
          Load
        </Button>
        {icao && (
          <Button size="sm" variant="outline" onClick={() => setConfigOpen(true)} className="md:hidden">
            <SlidersHorizontal />
            Config
          </Button>
        )}
        {!canEdit && <StatusPill tone="warn">View only — sign in with VATSIM to edit.</StatusPill>}
        {b?.metar && (
          <span className="flex min-w-0 items-center gap-2 font-mono text-xs">
            {b.flight_category && (
              <StatusPill tone={toneOf("category", b.flight_category)}>{b.flight_category}</StatusPill>
            )}
            {b.wind && <span className="text-ink">{b.wind}</span>}
            <span className="hidden truncate text-ink-3 lg:inline" title={b.metar}>
              {b.metar}
            </span>
          </span>
        )}
        {/* The top bar carries the clock from `sm` up. */}
        <ZuluClock className="ml-auto rounded-full border border-line bg-panel-2 px-2.5 py-1 text-xs text-ink-2 sm:hidden" />
      </div>

      {!icao ? (
        <EmptyState icon={PlaneLanding} className="flex-1">
          Load an airport to balance its arrival runways.
        </EmptyState>
      ) : (
        <div className="relative flex min-h-0 flex-1">
          {/* Config — a fixed side column on desktop, a bottom drawer on mobile. */}
          <Sheet
            open={configOpen}
            onClose={() => setConfigOpen(false)}
            className="w-80 shrink-0 border-r border-line"
            initialFraction={0.55}
          >
            <div className="flex flex-1 flex-col gap-5 overflow-y-auto p-4">
              <section className="flex flex-col gap-2">
                <h2 className="text-sm font-semibold">Landing runways</h2>
                {canEdit && (
                  <div className="flex items-center gap-1">
                    <Select
                      size="sm"
                      aria-label="Saved configs"
                      wrapperClassName="flex-1 min-w-0"
                      className="text-xs"
                      value={selectedCfg}
                      onChange={(e) => applyConfig(e.target.value)}
                    >
                      <option value="">— saved configs —</option>
                      {(configs.data ?? []).map((c) => (
                        <option key={c.name} value={c.name}>
                          {c.name}
                        </option>
                      ))}
                    </Select>
                    <Button size="sm" variant="outline" onClick={saveCurrentConfig}>
                      Save
                    </Button>
                    <ConfirmButton
                      size="icon"
                      className="size-8"
                      disabled={!selectedCfg}
                      aria-label="Delete saved config"
                      onConfirm={deleteSelectedConfig}
                      warn={selectedCfg ? `Delete the “${selectedCfg}” config?` : "Delete this config?"}
                    >
                      <X />
                    </ConfirmButton>
                  </div>
                )}
                <div className="flex flex-wrap gap-1.5">
                  {PRESETS.map((p) => (
                    <Button
                      key={p.id}
                      size="sm"
                      variant="outline"
                      className="h-7 px-2.5 text-xs"
                      disabled={!canEdit}
                      onClick={() => applyPreset(p.id)}
                    >
                      {p.label}
                    </Button>
                  ))}
                </div>
                <div className="flex flex-col gap-1.5">
                  {pairs.length === 0 && <p className="text-xs text-ink-3">No runway data for {b?.icao}.</p>}
                  {pairs.map(([pair, ends]) => (
                    <div key={pair} className="flex items-center gap-1.5">
                      {ends.map((e) => (
                        <button
                          key={e.id}
                          type="button"
                          aria-pressed={e.active}
                          disabled={!canEdit}
                          onClick={() => toggleEnd(e.id)}
                          className={`flex-1 rounded-xs border px-2 py-1 text-left font-mono text-xs transition-colors disabled:opacity-60 ${
                            e.active ? "border-brand bg-brand-soft text-ink" : "border-line text-ink-2 hover:bg-panel-2"
                          }`}
                        >
                          <span className="font-semibold">{e.id}</span>
                          <span className="ml-1 text-[10px] text-ink-3">{String(e.hdg).padStart(3, "0")}°</span>
                        </button>
                      ))}
                      <span className="w-14 shrink-0 text-right font-mono text-[10px] text-ink-3">
                        {ends.length === 1 && customIds.has(ends[0].id) && canEdit ? (
                          <button
                            type="button"
                            onClick={() => removeEnd(ends[0].id)}
                            className="hover:text-danger"
                            aria-label="Remove end"
                          >
                            × remove
                          </button>
                        ) : ends[0].len ? (
                          `${ends[0].len}ft`
                        ) : (
                          ""
                        )}
                      </span>
                    </div>
                  ))}
                </div>
                {canEdit && (
                  <div className="flex items-center gap-1">
                    <Input
                      aria-label="Runway end"
                      value={newEndId}
                      onChange={(e) => setNewEndId(e.target.value)}
                      placeholder="RWY"
                      className="h-8 w-16 font-mono text-xs uppercase"
                    />
                    <Input
                      aria-label="Heading"
                      value={newEndHdg}
                      onChange={(e) => setNewEndHdg(e.target.value)}
                      onKeyDown={(e) => e.key === "Enter" && addEnd()}
                      placeholder="HDG"
                      inputMode="numeric"
                      className="h-8 w-16 font-mono text-xs"
                    />
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={addEnd}
                      disabled={!newEndId.trim() || !newEndHdg.trim()}
                    >
                      Add end
                    </Button>
                  </div>
                )}
                <p className="text-[11px] leading-snug text-ink-3">
                  Presets pick ends by final-approach direction (WEST = landing westbound, 270 ± 65°). Add End
                  covers fields missing runway data.
                </p>
              </section>

              <section className="flex flex-col gap-2 border-t border-line pt-4">
                <h2 className="text-sm font-semibold">STAR → runway rules</h2>
                <div className="flex flex-col gap-1.5">
                  {Object.entries(b?.star_rules ?? {}).map(([star, rwy]) => (
                    <div key={star} className="flex items-center gap-2 text-xs">
                      <span className="flex-1 font-mono font-semibold">{star}</span>
                      <Select
                        size="sm"
                        aria-label={`Runway for ${star}`}
                        className="font-mono text-xs"
                        disabled={!canEdit}
                        value={rwy}
                        onChange={(e) => saveRules({ ...(b?.star_rules ?? {}), [star]: e.target.value })}
                      >
                        {activeIds.map((id) => (
                          <option key={id} value={id}>
                            {id}
                          </option>
                        ))}
                      </Select>
                      <button
                        type="button"
                        disabled={!canEdit}
                        onClick={() => removeRule(star)}
                        className="text-ink-3 hover:text-danger disabled:opacity-50"
                        aria-label="Remove rule"
                      >
                        <X className="size-3.5" />
                      </button>
                    </div>
                  ))}
                  {Object.keys(b?.star_rules ?? {}).length === 0 && (
                    <p className="text-[11px] text-ink-3">No rules — arrivals auto-balance.</p>
                  )}
                </div>
                {canEdit && (
                  <div className="flex items-center gap-1">
                    <Input
                      aria-label="STAR"
                      value={newStar}
                      onChange={(e) => setNewStar(e.target.value)}
                      onKeyDown={(e) => e.key === "Enter" && addRule()}
                      placeholder="STAR e.g. CAMRN"
                      className="h-8 min-w-0 flex-1 font-mono text-xs uppercase"
                    />
                    <Select
                      size="sm"
                      aria-label="Runway"
                      className="font-mono text-xs"
                      value={newRwy}
                      onChange={(e) => setNewRwy(e.target.value)}
                    >
                      <option value="">rwy</option>
                      {activeIds.map((id) => (
                        <option key={id} value={id}>
                          {id}
                        </option>
                      ))}
                    </Select>
                    <Button size="sm" variant="outline" onClick={addRule} disabled={!newStar.trim() || !newRwy}>
                      Pin
                    </Button>
                  </div>
                )}
                <p className="text-[11px] leading-snug text-ink-3">
                  A rule sends every arrival on that STAR to one runway. Aircraft overrides beat rules.
                </p>
              </section>

              <section className="flex flex-col gap-2 border-t border-line pt-4">
                <h2 className="text-sm font-semibold">Settings</h2>
                <label className="flex items-center gap-2 text-xs">
                  <span className="text-ink-2">Horizon</span>
                  <Select
                    size="sm"
                    disabled={!canEdit}
                    value={b?.window_min ?? 90}
                    onChange={(e) => setWindow(Number(e.target.value))}
                  >
                    {[60, 90, 120, 180].map((w) => (
                      <option key={w} value={w}>
                        {w} min
                      </option>
                    ))}
                  </Select>
                </label>
                <p className="text-[11px] leading-snug text-ink-3">
                  Config, rules, and assignments are shared across controllers. ETAs use a climb-profile + winds
                  model.
                </p>
              </section>
            </div>
          </Sheet>

          {/* Main — demand + arrivals */}
          <div className="flex min-w-0 flex-1 flex-col gap-6 overflow-y-auto p-4 sm:p-5">
            <DemandChart demand={b?.demand ?? []} bins={b?.bins ?? 9} />
            <ArrivalsByRunway
              ends={(b?.ends ?? []).filter((e) => e.active)}
              arrivals={arrivals}
              activeIds={activeIds}
              recMap={recMap}
              onOverride={setOverride}
              canEdit={canEdit}
            />
          </div>
        </div>
      )}
    </div>
  );
}

function groupPairs(ends: RunwayEnd[]): [string, RunwayEnd[]][] {
  const map = new Map<string, RunwayEnd[]>();
  for (const e of ends) {
    if (!map.has(e.pair)) map.set(e.pair, []);
    map.get(e.pair)!.push(e);
  }
  return [...map.entries()];
}

type Bin = { label: string; n: number; level: string };

function DemandChart({
  demand,
  bins,
}: {
  demand: { id: string; bins: number[]; levels: string[] }[];
  bins: number;
}) {
  const rows = useMemo(
    () =>
      demand.map((row) => ({
        id: row.id,
        total: row.bins.reduce((a, c) => a + c, 0),
        data: Array.from({ length: bins }, (_, i): Bin => ({
          label: `+${i * 10}m`,
          n: row.bins[i] ?? 0,
          level: row.levels[i],
        })),
      })),
    [demand, bins],
  );

  if (demand.length === 0) {
    return (
      <EmptyState className="rounded-md border border-dashed border-line">
        Select active landing runways to see arrival demand.
      </EmptyState>
    );
  }
  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-xl font-bold">Demand</h2>
        <span className="text-xs text-ink-3">current &amp; anticipated · 10-min bins · max {BIN_MAX}/bin</span>
      </div>
      <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
        {rows.map((row) => (
          <Card key={row.id} className="p-3">
            <div className="mb-1 flex items-baseline justify-between">
              <span className="font-mono text-lg font-bold">{row.id}</span>
              <span className="font-mono text-xs text-ink-3">{row.total} in window</span>
            </div>
            <Bars
              label={`${row.id} arrival demand, 10-minute bins`}
              data={row.data}
              category={(d) => d.label}
              value={(d) => d.n}
              color={(d) => LEVEL_TOKEN[d.level] ?? "level-ok"}
              cap={BIN_MAX}
              valueFormat={(v) => (Number.isInteger(v) ? String(v) : "")}
              height={120}
            />
          </Card>
        ))}
      </div>
    </section>
  );
}

type Group = { id: string; hdg?: number; list: RunwayArrival[] };

type AssignContext = {
  activeIds: string[];
  recMap: Map<string, { to_rwy: string; level: string }>;
  onOverride: (cs: string, rwy: string) => void;
  canEdit: boolean;
};

// The board refetches every 15s. Cells read the live assignment state from context so the column
// definitions stay module-constant — a new cell function would remount the override Select and close
// it (or drop focus) mid-choice.
const AssignCtx = createContext<AssignContext | null>(null);

function AssignCell({ a }: { a: RunwayArrival }) {
  const { activeIds, recMap, onOverride, canEdit } = useContext(AssignCtx)!;
  const rec = recMap.get(a.cs);
  return (
    <div className="flex items-center justify-end gap-1.5">
      {rec && (
        <span title={`Rebalance: move to ${rec.to_rwy}`}>
          <StatusPill tone={toneOf("level", rec.level)} className="px-1.5 font-mono text-[10px] leading-4">
            → {rec.to_rwy}
          </StatusPill>
        </span>
      )}
      <StatusPill tone={a.src === "man" ? "brand" : "neutral"} className="px-1.5 font-mono text-[10px] uppercase leading-4">
        {a.src}
      </StatusPill>
      <Select
        size="sm"
        aria-label={`Runway override for ${a.cs}`}
        className="h-7 font-mono text-[11px]"
        disabled={!canEdit}
        value={a.src === "man" ? (a.rwy ?? "AUTO") : "AUTO"}
        onChange={(e) => onOverride(a.cs, e.target.value)}
      >
        <option value="AUTO">AUTO</option>
        {activeIds.map((id) => (
          <option key={id} value={id}>
            {id}
          </option>
        ))}
      </Select>
    </div>
  );
}

const ARRIVAL_COLUMNS: DataColumn<RunwayArrival>[] = [
  { accessorKey: "cs", header: "Callsign", mono: true, cellClassName: "px-2 font-semibold" },
  {
    id: "route",
    accessorFn: (a) => `${a.dep} ${a.star ?? ""}`,
    header: "Route",
    mono: true,
    cellClassName: "px-2 text-xs text-ink-2",
    cell: (c) => (
      <span className="whitespace-nowrap">
        {c.row.original.dep} · {c.row.original.star ?? "—"}
      </span>
    ),
  },
  {
    accessorKey: "eta",
    header: "ETA",
    mono: true,
    cellClassName: "px-2 text-xs",
    cell: (c) => (
      <span className="whitespace-nowrap">
        {zulu(c.row.original.eta)} <span className="text-ink-3">+{minsFromNow(c.row.original.eta)}</span>
      </span>
    ),
  },
  {
    id: "assign",
    header: "Assignment",
    align: "right",
    enableSorting: false,
    cellClassName: "px-2",
    cell: (c) => <AssignCell a={c.row.original} />,
  },
];

function ArrivalsByRunway({
  ends,
  arrivals,
  activeIds,
  recMap,
  onOverride,
  canEdit,
}: {
  ends: RunwayEnd[];
  arrivals: RunwayArrival[];
  activeIds: string[];
  recMap: Map<string, { to_rwy: string; level: string }>;
  onOverride: (cs: string, rwy: string) => void;
  canEdit: boolean;
}) {
  const groups: Group[] = ends.map((e) => ({
    id: e.id,
    hdg: e.hdg,
    list: arrivals.filter((a) => a.rwy === e.id),
  }));
  const unassigned = arrivals.filter((a) => !a.rwy || !ends.some((e) => e.id === a.rwy));
  if (unassigned.length) groups.push({ id: "unassigned", list: unassigned });

  return (
    <AssignCtx.Provider value={{ activeIds, recMap, onOverride, canEdit }}>
    <section className="flex flex-col gap-3">
      <h2 className="text-xl font-bold">Arrivals by runway</h2>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2 2xl:grid-cols-3">
        {groups.map((g) => (
          <div key={g.id} className="flex min-w-0 flex-col gap-2">
            <div className="flex items-baseline justify-between px-1">
              <span className="font-mono text-sm font-bold">
                {g.id === "unassigned" ? "UNASSIGNED" : g.id}
                {g.hdg != null && (
                  <span className="ml-1.5 text-[10px] font-normal text-ink-3">{String(g.hdg).padStart(3, "0")}°</span>
                )}
              </span>
              <span className="font-mono text-xs text-ink-3">{g.list.length}</span>
            </div>
            <DataTable
              label={`Arrivals for ${g.id}`}
              columns={ARRIVAL_COLUMNS}
              data={g.list}
              getRowId={(a) => a.cs}
              hideHeader
              // Live ops list: always pages, never hides rows behind "Show all".
              rowCap={Infinity}
              pageSize={25}
              rowClassName={(a) => (recMap.has(a.cs) ? "bg-warning-soft" : undefined)}
              empty="No arrivals"
            />
          </div>
        ))}
      </div>
    </section>
    </AssignCtx.Provider>
  );
}

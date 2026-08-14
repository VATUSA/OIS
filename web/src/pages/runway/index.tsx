import {useMemo, useState} from "react";
import {Button, Input} from "@ois/ui";

import {ZuluClock} from "@/components/zulu-clock";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type RunwayArrival, type RunwayEnd, useRunway, useUpdateRunway,} from "@/lib/runway";

const BIN_MAX = 5;
const PRESET_HDG: Record<string, number> = { W: 270, E: 90, N: 360, S: 180 };
const LEVEL_COLOR: Record<string, string> = {
  green: "#4caf7a",
  yellow: "#e8a838",
  red: "#d45c5c",
};

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
  const board = useRunway(icao);
  const update = useUpdateRunway(icao ?? "");
  const b = board.data;

  const activeIds = useMemo(
    () => (b?.ends ?? []).filter((e) => e.active).map((e) => e.id),
    [b?.ends],
  );

  function save(active: string[], windowMin?: number) {
    if (!icao || !b) return;
    update.mutate({
      active_ends: active,
      star_rules: b.star_rules,
      overrides: b.overrides,
      window_min: windowMin ?? b.window_min,
    });
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
    update.mutate({
      active_ends: activeIds,
      star_rules: rules,
      overrides: b.overrides,
      window_min: b.window_min,
    });
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
    update.mutate({
      active_ends: activeIds,
      star_rules: b.star_rules,
      overrides: pruned,
      window_min: b.window_min,
    });
  }

  const load = () => {
    const f = field.trim().toUpperCase();
    if (f) setIcao(f);
  };

  if (!canRead) {
    return (
      <div className="flex h-[calc(100vh-3.5rem)] items-center justify-center text-sm text-muted-foreground">
        You don&apos;t have flow access.
      </div>
    );
  }

  const arrivals = b?.arrivals ?? [];
  const assigned = arrivals.filter((a) => a.rwy);
  const pairs = groupPairs(b?.ends ?? []);
  const recMap = new Map((b?.recs ?? []).map((r) => [r.cs, r]));

  return (
    <div className="flex h-[calc(100vh-3.5rem)] flex-col">
      {/* Header */}
      <div className="flex items-center gap-4 border-b px-5 py-2.5 text-sm">
        <span className="font-semibold uppercase tracking-wider">
          Runway <span className="text-primary">Balancer</span>
        </span>
        <Input
          value={field}
          onChange={(e) => setField(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && load()}
          placeholder="ICAO"
          className="h-8 w-28 font-mono uppercase"
        />
        <Button size="sm" onClick={load} disabled={!field.trim()}>
          Load
        </Button>
        {b && (
          <span className="flex items-center gap-3 text-xs text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <span className="size-1.5 rounded-full bg-emerald-500" /> live
            </span>
            <span>
              arrivals <b className="text-foreground">{arrivals.length}</b>
            </span>
            <span>
              assigned <b className="text-foreground">{assigned.length}</b>
            </span>
            <span>
              {b.icao} rwy data:{" "}
              <span className="text-foreground">{b.source}</span>
            </span>
          </span>
        )}
        {!canEdit && (
          <span className="rounded border border-amber-500/50 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-200">
            View only — sign in with VATSIM to edit.
          </span>
        )}
        <ZuluClock className="ml-auto rounded-md border bg-muted/40 px-2 py-1 font-mono text-muted-foreground" />
      </div>

      {!icao ? (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          Load an airport to balance its arrival runways.
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          {/* Sidebar — config */}
          <aside className="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto border-r p-4">
            <section>
              <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Landing runways
              </h2>
              <div className="mb-2 flex flex-wrap gap-1.5">
                {["W", "E", "N", "S", "OFF"].map((p) => (
                  <button
                    key={p}
                    type="button"
                    disabled={!canEdit}
                    onClick={() => applyPreset(p)}
                    className="rounded border px-2 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50"
                  >
                    {{ W: "WEST", E: "EAST", N: "NORTH", S: "SOUTH", OFF: "NONE" }[p]}
                  </button>
                ))}
              </div>
              <div className="flex flex-col gap-1.5">
                {pairs.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    No runway data for {b?.icao}.
                  </p>
                )}
                {pairs.map(([pair, ends]) => (
                  <div key={pair} className="flex items-center gap-1.5">
                    {ends.map((e) => (
                      <button
                        key={e.id}
                        type="button"
                        disabled={!canEdit}
                        onClick={() => toggleEnd(e.id)}
                        className={`flex-1 rounded border px-2 py-1 text-left font-mono text-xs transition-colors disabled:opacity-60 ${
                          e.active
                            ? "border-primary bg-primary/15 text-foreground"
                            : "text-muted-foreground hover:bg-accent"
                        }`}
                      >
                        <span className="font-semibold">{e.id}</span>
                        <span className="ml-1 text-[10px] text-muted-foreground">
                          {String(e.hdg).padStart(3, "0")}°
                        </span>
                      </button>
                    ))}
                    <span className="w-14 shrink-0 text-right font-mono text-[10px] text-muted-foreground">
                      {ends[0].len ? `${ends[0].len}ft` : ""}
                    </span>
                  </div>
                ))}
              </div>
              <p className="mt-2 text-[11px] leading-snug text-muted-foreground">
                Presets pick ends by final-approach direction (WEST = landing
                westbound, 270 ± 65°). Click chips to toggle individual ends.
              </p>
            </section>

            <section>
              <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                STAR → runway rules
              </h2>
              <div className="flex flex-col gap-1.5">
                {Object.entries(b?.star_rules ?? {}).map(([star, rwy]) => (
                  <div key={star} className="flex items-center gap-2 text-xs">
                    <span className="flex-1 font-mono font-semibold">{star}</span>
                    <select
                      disabled={!canEdit}
                      value={rwy}
                      onChange={(e) =>
                        saveRules({ ...(b?.star_rules ?? {}), [star]: e.target.value })
                      }
                      className="h-7 rounded border border-input bg-background px-1 font-mono text-xs outline-none disabled:opacity-50"
                    >
                      {activeIds.map((id) => (
                        <option key={id} value={id}>
                          {id}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      disabled={!canEdit}
                      onClick={() => removeRule(star)}
                      className="text-muted-foreground hover:text-destructive disabled:opacity-50"
                      aria-label="Remove rule"
                    >
                      ×
                    </button>
                  </div>
                ))}
                {Object.keys(b?.star_rules ?? {}).length === 0 && (
                  <p className="text-[11px] text-muted-foreground">
                    No rules — arrivals auto-balance.
                  </p>
                )}
              </div>
              {canEdit && (
                <div className="mt-2 flex items-center gap-1">
                  <Input
                    value={newStar}
                    onChange={(e) => setNewStar(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && addRule()}
                    placeholder="STAR e.g. CAMRN"
                    className="h-7 flex-1 font-mono text-xs uppercase"
                  />
                  <select
                    value={newRwy}
                    onChange={(e) => setNewRwy(e.target.value)}
                    className="h-7 rounded border border-input bg-background px-1 font-mono text-xs outline-none"
                  >
                    <option value="">rwy</option>
                    {activeIds.map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </select>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={addRule}
                    disabled={!newStar.trim() || !newRwy}
                    className="h-7 px-2 text-xs"
                  >
                    Pin
                  </Button>
                </div>
              )}
              <p className="mt-2 text-[11px] leading-snug text-muted-foreground">
                A rule sends every arrival on that STAR to one runway. Aircraft
                overrides beat rules.
              </p>
            </section>

            <section>
              <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Settings
              </h2>
              <label className="flex items-center gap-2 text-xs">
                <span className="text-muted-foreground">horizon</span>
                <select
                  disabled={!canEdit}
                  value={b?.window_min ?? 90}
                  onChange={(e) => save(activeIds, Number(e.target.value))}
                  className="h-8 rounded-md border border-input bg-background px-2 text-sm outline-none disabled:opacity-50"
                >
                  {[60, 90, 120, 180].map((w) => (
                    <option key={w} value={w}>
                      {w} min
                    </option>
                  ))}
                </select>
              </label>
              <p className="mt-2 text-[11px] leading-snug text-muted-foreground">
                Config, rules, and assignments are shared across controllers. ETAs
                use a climb-profile + winds model.
              </p>
            </section>
          </aside>

          {/* Main — demand + arrivals */}
          <div className="min-w-0 flex-1 overflow-y-auto p-4">
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

function DemandChart({
  demand,
  bins,
}: {
  demand: { id: string; bins: number[]; levels: string[] }[];
  bins: number;
}) {
  if (demand.length === 0) {
    return (
      <div className="mb-6 rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
        Select active landing runways to see arrival demand.
      </div>
    );
  }
  return (
    <div className="mb-6">
      <h2 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        Demand — current &amp; anticipated · 10-min bins · max {BIN_MAX}/bin
      </h2>
      <div className="flex flex-col gap-2">
        {demand.map((row) => {
          const total = row.bins.reduce((a, c) => a + c, 0);
          return (
            <div key={row.id} className="flex items-center gap-3">
              <div className="w-24 shrink-0">
                <div className="font-mono text-lg font-bold">{row.id}</div>
                <div className="text-[11px] text-muted-foreground">
                  {total} in window
                </div>
              </div>
              <div className="flex flex-1 gap-1">
                {Array.from({ length: bins }, (_, i) => {
                  const n = row.bins[i] ?? 0;
                  const h = Math.min(n, BIN_MAX) / BIN_MAX;
                  return (
                    <div
                      key={i}
                      className="relative flex h-14 flex-1 items-end rounded border border-border/40 bg-muted/10"
                      title={`+${i * 10}–${i * 10 + 10} min: ${n}`}
                    >
                      {n > 0 && (
                        <div
                          className="w-full rounded-b"
                          style={{
                            height: `${Math.max(h * 100, 8)}%`,
                            background: LEVEL_COLOR[row.levels[i]] ?? LEVEL_COLOR.green,
                          }}
                        />
                      )}
                      {n > 0 && (
                        <span className="absolute inset-x-0 top-1 text-center font-mono text-[11px] font-semibold">
                          {n}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
      <div className="mt-1 flex gap-1 pl-[6.75rem] font-mono text-[10px] text-muted-foreground">
        {Array.from({ length: bins }, (_, i) => (
          <span key={i} className="flex-1">
            {i % 3 === 0 ? `+${i * 10}m` : ""}
          </span>
        ))}
      </div>
    </div>
  );
}

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
  const groups: { id: string; hdg?: number; list: RunwayArrival[] }[] = ends.map(
    (e) => ({
      id: e.id,
      hdg: e.hdg,
      list: arrivals.filter((a) => a.rwy === e.id),
    }),
  );
  const unassigned = arrivals.filter((a) => !a.rwy || !ends.some((e) => e.id === a.rwy));
  if (unassigned.length) groups.push({ id: "unassigned", list: unassigned });

  return (
    <div>
      <h2 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        Arrivals by runway
      </h2>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
        {groups.map((g) => (
          <div key={g.id} className="rounded-lg border">
            <div className="flex items-center justify-between border-b px-3 py-2">
              <span className="font-mono text-sm font-bold">
                {g.id === "unassigned" ? "UNASSIGNED" : g.id}
                {g.hdg != null && (
                  <span className="ml-1.5 text-[10px] font-normal text-muted-foreground">
                    {String(g.hdg).padStart(3, "0")}°
                  </span>
                )}
              </span>
              <span className="font-mono text-xs text-muted-foreground">
                {g.list.length}
              </span>
            </div>
            {g.list.length === 0 ? (
              <div className="px-3 py-4 text-center text-xs text-muted-foreground">
                No arrivals
              </div>
            ) : (
              <table className="w-full text-xs">
                <tbody>
                  {g.list.map((a) => {
                    const rec = recMap.get(a.cs);
                    return (
                      <tr
                        key={a.cs}
                        className={`border-b last:border-0 ${rec ? "bg-amber-500/5" : ""}`}
                      >
                        <td className="px-3 py-1.5 font-mono font-semibold">{a.cs}</td>
                        <td className="py-1.5 font-mono text-muted-foreground">{a.dep}</td>
                        <td className="py-1.5 font-mono text-muted-foreground">
                          {a.star ?? "—"}
                        </td>
                        <td className="py-1.5 font-mono">
                          {zulu(a.eta)}{" "}
                          <span className="text-muted-foreground">
                            +{minsFromNow(a.eta)}
                          </span>
                        </td>
                        <td className="px-3 py-1.5">
                          <div className="flex items-center justify-end gap-1.5">
                            {rec && (
                              <span
                                className="rounded px-1 py-0.5 font-mono text-[10px] font-semibold"
                                title={`Rebalance: move to ${rec.to_rwy}`}
                                style={{
                                  color: LEVEL_COLOR[rec.level] ?? "#888",
                                  background: `${LEVEL_COLOR[rec.level] ?? "#888"}22`,
                                }}
                              >
                                → {rec.to_rwy}
                              </span>
                            )}
                            <span
                              className={`rounded px-1 py-0.5 font-mono text-[10px] uppercase ${
                                a.src === "man"
                                  ? "bg-sky-500/15 text-sky-500"
                                  : a.src === "star"
                                    ? "bg-violet-500/15 text-violet-500"
                                    : "bg-muted text-muted-foreground"
                              }`}
                            >
                              {a.src}
                            </span>
                            <select
                              disabled={!canEdit}
                              value={a.src === "man" ? (a.rwy ?? "AUTO") : "AUTO"}
                              onChange={(e) => onOverride(a.cs, e.target.value)}
                              className="h-6 rounded border border-input bg-background px-1 font-mono text-[10px] outline-none disabled:opacity-50"
                            >
                              <option value="AUTO">AUTO</option>
                              {activeIds.map((id) => (
                                <option key={id} value={id}>
                                  {id}
                                </option>
                              ))}
                            </select>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

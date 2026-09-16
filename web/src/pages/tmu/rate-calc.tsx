import {useEffect, useState} from "react";
import {Button, Card, Input, MetricCard, Select, type Tone} from "@ois/ui";
import {Gauge} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {POSITIONS, type Positions, recommendedAar, tierForIcao, type TierKey, TIERS,} from "@/lib/rate-calc";
import {usePrograms, useUpsertProgram} from "@/lib/tmu";

const STORE_KEY = "ois.ratecalc";

type Stored = {
  icao: string;
  tier: TierKey;
  maxAar: number;
  fullStaff: number;
  onDuty: number;
  usePositions: boolean;
  advancedOpen: boolean;
  positions: Positions;
};

const DEFAULT: Stored = {
  icao: "",
  tier: "hub",
  maxAar: 120,
  fullStaff: 6,
  onDuty: 2,
  usePositions: false,
  advancedOpen: false,
  positions: { app: true, twr: true, gnd: false, del: false, sup: false, atis: false },
};

function load(): Stored {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return raw ? { ...DEFAULT, ...(JSON.parse(raw) as Stored) } : DEFAULT;
  } catch {
    return DEFAULT;
  }
}

function Field({
  label,
  children,
  width = "w-20",
}: {
  label: string;
  children: React.ReactNode;
  width?: string;
}) {
  return (
    <label
      className={`flex flex-col gap-1 text-xs font-semibold text-ink-2 ${width}`}
    >
      {label}
      {children}
    </label>
  );
}

export function RateCalculatorTab() {
  const { data: me } = useMe();
  const canApply = hasPermission(me, "tmu.program.update");
  const programs = usePrograms();
  const upsert = useUpsertProgram();
  const [s, setS] = useState<Stored>(load);

  useEffect(() => {
    localStorage.setItem(STORE_KEY, JSON.stringify(s));
  }, [s]);

  function set<K extends keyof Stored>(key: K, value: Stored[K]) {
    setS((prev) => ({ ...prev, [key]: value }));
  }
  function applyTier(tier: TierKey) {
    const t = TIERS[tier];
    setS((prev) => ({ ...prev, tier, maxAar: t.maxAar, fullStaff: t.fullStaff }));
  }
  function onIcao(raw: string) {
    const icao = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    setS((prev) => {
      const next = { ...prev, icao };
      if (icao.length === 4) {
        const tier = tierForIcao(icao);
        const t = TIERS[tier];
        return { ...next, tier, maxAar: t.maxAar, fullStaff: t.fullStaff };
      }
      return next;
    });
  }

  const result = recommendedAar({
    maxAar: s.maxAar,
    fullStaff: s.fullStaff,
    onDuty: s.onDuty,
    usePositions: s.usePositions,
    positions: s.positions,
  });

  const validIcao = /^[A-Z0-9]{4}$/.test(s.icao);
  const resultTone: Tone | undefined =
    result.aar <= 0 ? undefined : result.pct >= 70 ? "good" : result.pct >= 40 ? "warn" : "bad";
  const limitedTxt =
    result.limitedBy === "positions"
      ? "position mix"
      : result.limitedBy === "critical"
        ? "missing APP/TWR"
        : result.limitedBy === "staff"
          ? "no staff"
          : "headcount";

  function apply() {
    const existing = programs.data?.find((p) => p.icao === s.icao);
    const body = existing
      ? {
          aar: result.aar,
          trail: existing.trail,
          mit: existing.mit,
          gates: existing.gates,
          exclude_wake: existing.exclude_wake,
          exclude_types: existing.exclude_types,
          jets_only: existing.jets_only,
          active_until: existing.active_until,
        }
      : {
          aar: result.aar,
          trail: 0,
          mit: 0,
          gates: [],
          exclude_wake: [],
          exclude_types: [],
          jets_only: false,
          active_until: null,
        };
    upsert.mutate({ icao: s.icao, body });
  }

  return (
    <div className="flex flex-col gap-6">
      {/* control row */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-end gap-3">
          <Field label="Airport" width="w-28">
            <Input
              className="font-mono uppercase"
              maxLength={4}
              placeholder="KJFK"
              value={s.icao}
              onChange={(e) => onIcao(e.target.value)}
            />
          </Field>
          <Field label="Tier" width="w-56">
            <Select value={s.tier} onChange={(e) => applyTier(e.target.value as TierKey)}>
              {Object.entries(TIERS).map(([k, t]) => (
                <option key={k} value={k}>
                  {t.label} ({t.maxAar}/hr, {t.fullStaff} staff)
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Max AAR">
            <Input
              className="font-mono"
              type="number"
              min={1}
              max={200}
              value={s.maxAar}
              onChange={(e) => set("maxAar", Number(e.target.value))}
            />
          </Field>
          <Field label="Full staff" width="w-24">
            <Input
              className="font-mono"
              type="number"
              min={1}
              max={20}
              value={s.fullStaff}
              onChange={(e) => set("fullStaff", Number(e.target.value))}
            />
          </Field>
          <Field label="On duty" width="w-24">
            <Input
              className="font-mono"
              type="number"
              min={0}
              max={20}
              value={s.onDuty}
              onChange={(e) => set("onDuty", Number(e.target.value))}
            />
          </Field>
          <Button variant="ghost" size="sm" className="mb-0.5" onClick={() => set("advancedOpen", !s.advancedOpen)}>
            {s.advancedOpen ? "▾ Hide" : "▸ Show"} position checklist
          </Button>
        </div>

        {s.advancedOpen && (
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-sm border border-line bg-panel-2 px-3 py-2">
            <label className="flex w-fit items-center gap-2 text-sm font-semibold">
              <input
                type="checkbox"
                className="size-3.5 accent-brand"
                checked={s.usePositions}
                onChange={(e) => set("usePositions", e.target.checked)}
              />
              Use position weights
            </label>
            {POSITIONS.map((p) => (
              <label key={p.key} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  className="size-3.5 accent-brand"
                  disabled={!s.usePositions}
                  checked={s.positions[p.key]}
                  onChange={(e) => set("positions", { ...s.positions, [p.key]: e.target.checked })}
                />
                {p.label}
              </label>
            ))}
          </div>
        )}
      </div>

      {/* result */}
      <div className="flex flex-col gap-3">
        <MetricCard
          label="Recommended AAR"
          icon={Gauge}
          tone={resultTone}
          className="max-w-md"
          value={
            result.aar > 0 ? (
              <>
                {result.aar}
                <span className="text-base font-semibold text-ink-3"> /hr</span>
              </>
            ) : (
              <span className="font-sans text-xl text-ink-2">{result.warning ?? "—"}</span>
            )
          }
          sub={
            result.aar > 0 ? (
              <>
                <span className="font-mono">{result.pct}%</span> of max capacity ·{" "}
<span className="font-mono">{s.onDuty}</span> of <span className="font-mono">{s.fullStaff}</span>{" "}
                controllers · limited by {limitedTxt}
              </>
            ) : undefined
          }
        />
        {result.aar > 0 && (
          <p className="font-mono text-xs text-ink-3">
            maxAAR × (onDuty / fullStaff) = {s.maxAar} × ({s.onDuty} / {s.fullStaff}) = {result.aar}/hr
          </p>
        )}

        {result.aar > 0 && validIcao && (
          <div className="flex flex-wrap items-center gap-3">
            {canApply ? (
              <>
                <Button disabled={upsert.isPending} onClick={apply}>
                  Apply to <span className="font-mono">{s.icao}</span>
                </Button>
                <span className="text-xs text-ink-3">Sets the AAR on the Programs tab (keeps trail/MIT/gates).</span>
              </>
            ) : (
              <span className="text-xs text-ink-3">View only — you can&apos;t set programs.</span>
            )}
          </div>
        )}
      </div>

      <Card className="p-4 text-sm text-ink-2">
        <h2 className="mb-2 text-xl font-bold text-ink">How it works</h2>
        <ul className="list-disc space-y-1 pl-5">
          <li>Pick an airport tier to pre-fill max AAR and full staffing, or set your own.</li>
          <li>Recommended AAR = max AAR × (controllers on duty ÷ full staffing).</li>
          <li>A hub at 120/hr with 6 full positions and 2 on duty → 40/hr.</li>
          <li>
            Enable the position checklist to cap capacity when critical positions (APP, TWR)
            aren&apos;t staffed.
          </li>
        </ul>
      </Card>
    </div>
  );
}

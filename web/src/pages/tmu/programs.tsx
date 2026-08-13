import {useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {timeAgo} from "@/lib/time";
import {type GateRule, type Program, useDeleteProgram, usePrograms, useUpsertProgram,} from "@/lib/tmu";

const TRAIL_OPTS: [number, string][] = [
  [0, "AUTO"],
  [3, "3 min"],
  [5, "5 min"],
  [7, "7 min"],
  [10, "10 min"],
  [15, "15 min"],
  [20, "20 min"],
];

const SELECT =
  "h-9 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

type Draft = {
  aar: number;
  trail: number;
  mit: number;
  gates: GateRule[];
  exclude_wake: string[];
  exclude_types: string[];
  jets_only: boolean;
};

function toDraft(p: Program): Draft {
  return {
    aar: p.aar,
    trail: p.trail,
    mit: p.mit,
    gates: p.gates.map((g) => ({ name: g.name, trail: g.trail ?? 0, mit: g.mit ?? 0 })),
    exclude_wake: [...p.exclude_wake],
    exclude_types: [...p.exclude_types],
    jets_only: p.jets_only,
  };
}

function parseTypes(v: string): string[] {
  return v
    .split(/[,\s]+/)
    .map((s) => s.toUpperCase().replace(/[^A-Z0-9]/g, ""))
    .filter((s) => s.length >= 2 && s.length <= 4);
}

function spacingLabel(trail: number, mit: number): string {
  if (mit > 0) return `${mit} MIT`;
  if (trail > 0) return `${trail} MIN`;
  return "AUTO";
}

function TrailSelect({
  value,
  onChange,
  disabled,
}: {
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <select
      className={SELECT}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(Number(e.target.value))}
    >
      {TRAIL_OPTS.map(([v, label]) => (
        <option key={v} value={v}>
          {label}
        </option>
      ))}
    </select>
  );
}

// --- add a new airport program ---

function SetProgramForm() {
  const upsert = useUpsertProgram();
  const [icao, setIcao] = useState("");
  const [aar, setAar] = useState("30");
  const [trail, setTrail] = useState(0);
  const [mit, setMit] = useState("");
  const [error, setError] = useState<string | null>(null);

  function submit() {
    const cleanIcao = icao.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (cleanIcao.length < 3 || cleanIcao.length > 4) {
      setError("Enter a 3–4 character ICAO.");
      return;
    }
    const aarN = Math.round(Number(aar));
    if (!Number.isFinite(aarN) || aarN < 1 || aarN > 200) {
      setError("AAR must be between 1 and 200.");
      return;
    }
    const mitN = mit.trim() ? Math.round(Number(mit)) : 0;
    setError(null);
    upsert.mutate(
      {
        icao: cleanIcao,
        body: {
          aar: aarN,
          trail: mitN > 0 ? 0 : trail,
          mit: mitN,
          gates: [],
          exclude_wake: [],
          exclude_types: [],
          jets_only: false,
        },
      },
      {
        onSuccess: () => {
          setIcao("");
          setAar("30");
          setTrail(0);
          setMit("");
        },
      },
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-wrap items-end gap-3 pt-6">
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Airport
          <Input
            className="w-24 font-mono uppercase"
            maxLength={4}
            placeholder="KORD"
            value={icao}
            onChange={(e) => setIcao(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          AAR
          <Input
            className="w-20"
            type="number"
            min={1}
            max={200}
            value={aar}
            onChange={(e) => setAar(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Route trail
          <TrailSelect value={trail} onChange={setTrail} disabled={!!mit.trim()} />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          MIT (nm)
          <Input
            className="w-24"
            type="number"
            min={0}
            max={300}
            placeholder="override"
            value={mit}
            onChange={(e) => setMit(e.target.value)}
          />
        </label>
        <Button disabled={upsert.isPending} onClick={submit}>
          <Plus />
          Set program
        </Button>
        <p className="w-full text-sm text-destructive">{error ?? ""}</p>
      </CardContent>
    </Card>
  );
}

// --- one airport program (editable) ---

function ProgramCard({
  program,
  canEdit,
  canDelete,
}: {
  program: Program;
  canEdit: boolean;
  canDelete: boolean;
}) {
  const upsert = useUpsertProgram();
  const del = useDeleteProgram();

  const server = toDraft(program);
  const [draft, setDraft] = useState<Draft>(server);
  const [typesStr, setTypesStr] = useState(server.exclude_types.join(", "));
  // Reset local edits when the server record changes (e.g. after a save).
  const [syncKey, setSyncKey] = useState(program.updated_at);
  if (program.updated_at !== syncKey) {
    setSyncKey(program.updated_at);
    setDraft(server);
    setTypesStr(server.exclude_types.join(", "));
  }

  const dirty = JSON.stringify(draft) !== JSON.stringify(server);

  function patch(p: Partial<Draft>) {
    setDraft((d) => ({ ...d, ...p }));
  }
  function patchGate(i: number, p: Partial<GateRule>) {
    setDraft((d) => ({
      ...d,
      gates: d.gates.map((g, gi) => (gi === i ? { ...g, ...p } : g)),
    }));
  }

  const wakeL = draft.exclude_wake.includes("L");

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        {/* header */}
        <div className="flex flex-wrap items-center gap-4">
          <span className="font-mono text-lg font-semibold">{program.icao}</span>
          <div className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
            {program.updated_by && (
              <span>
                {program.updated_by} · {timeAgo(program.updated_at)}
              </span>
            )}
            {canDelete && (
              <Button
                size="sm"
                variant="ghost"
                className="text-destructive hover:text-destructive"
                disabled={del.isPending}
                onClick={() => del.mutate(program.icao)}
              >
                Remove
              </Button>
            )}
          </div>
        </div>

        {/* airport-wide spacing */}
        <div className="flex flex-wrap items-end gap-4">
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            AAR
            {canEdit ? (
              <Input
                className="w-20"
                type="number"
                min={1}
                max={200}
                value={draft.aar}
                onChange={(e) => patch({ aar: Number(e.target.value) })}
              />
            ) : (
              <span className="text-base text-foreground">{draft.aar}/hr</span>
            )}
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Route trail
            {canEdit ? (
              <TrailSelect
                value={draft.trail}
                disabled={draft.mit > 0}
                onChange={(v) => patch({ trail: v, mit: 0 })}
              />
            ) : (
              <span className="text-base text-foreground">
                {draft.trail > 0 ? `${draft.trail} MIN` : "AUTO"}
              </span>
            )}
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            MIT (nm)
            {canEdit ? (
              <Input
                className="w-24"
                type="number"
                min={0}
                max={300}
                placeholder="override"
                value={draft.mit || ""}
                onChange={(e) =>
                  patch({ mit: Math.max(0, Number(e.target.value) || 0) })
                }
              />
            ) : (
              <span className="text-base text-foreground">
                {draft.mit > 0 ? `${draft.mit} MIT` : "—"}
              </span>
            )}
          </label>
          <span className="pb-2 text-xs text-muted-foreground">
            airport-wide default · everything else uses this unless a gate below
            overrides it
          </span>
        </div>

        {/* per-gate restrictions */}
        <div className="flex flex-col gap-2 border-t pt-3">
          <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Gate restrictions
          </span>
          {draft.gates.length === 0 && (
            <span className="text-sm text-muted-foreground">
              None — all arrivals use the airport-wide spacing.
            </span>
          )}
          {draft.gates.map((g, i) => (
            <div key={i} className="flex flex-wrap items-center gap-2">
              <span className="text-muted-foreground">↳</span>
              {canEdit ? (
                <>
                  <Input
                    className="w-32 font-mono uppercase"
                    maxLength={8}
                    placeholder="JJEDI4"
                    value={g.name}
                    onChange={(e) => patchGate(i, { name: e.target.value })}
                  />
                  <TrailSelect
                    value={g.trail ?? 0}
                    disabled={(g.mit ?? 0) > 0}
                    onChange={(v) => patchGate(i, { trail: v, mit: 0 })}
                  />
                  <Input
                    className="w-24"
                    type="number"
                    min={0}
                    max={300}
                    placeholder="MIT nm"
                    value={g.mit || ""}
                    onChange={(e) =>
                      patchGate(i, { mit: Math.max(0, Number(e.target.value) || 0) })
                    }
                  />
                  <span className="text-xs text-muted-foreground">
                    {spacingLabel(g.trail ?? 0, g.mit ?? 0)}
                  </span>
                  <button
                    type="button"
                    title="Remove gate"
                    onClick={() =>
                      patch({ gates: draft.gates.filter((_, gi) => gi !== i) })
                    }
                    className="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:text-destructive"
                  >
                    <X className="size-4" />
                  </button>
                </>
              ) : (
                <span className="text-sm">
                  <b className="font-mono">{g.name}</b> —{" "}
                  {spacingLabel(g.trail ?? 0, g.mit ?? 0)}
                </span>
              )}
            </div>
          ))}
          {canEdit && draft.gates.length < 10 && (
            <button
              type="button"
              onClick={() =>
                patch({
                  gates: [...draft.gates, { name: "", trail: 0, mit: 0 }],
                })
              }
              className="w-fit text-xs font-medium uppercase tracking-wide text-primary hover:underline"
            >
              + Add gate ({draft.gates.length}/10)
            </button>
          )}
        </div>

        {/* aircraft exclusions */}
        <div className="flex flex-wrap items-center gap-4 border-t pt-3">
          <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Exclude
          </span>
          {canEdit ? (
            <>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={wakeL}
                  onChange={(e) =>
                    patch({
                      exclude_wake: e.target.checked
                        ? [...new Set([...draft.exclude_wake, "L"])]
                        : draft.exclude_wake.filter((w) => w !== "L"),
                    })
                  }
                />
                Wake L
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.jets_only}
                  onChange={(e) => patch({ jets_only: e.target.checked })}
                />
                Jets / turbines only
              </label>
              <label className="flex items-center gap-2 text-sm text-muted-foreground">
                Types
                <Input
                  className="w-48 font-mono uppercase"
                  placeholder="C172, PA28"
                  value={typesStr}
                  onChange={(e) => {
                    setTypesStr(e.target.value);
                    patch({ exclude_types: parseTypes(e.target.value) });
                  }}
                />
              </label>
            </>
          ) : (
            <span className="text-sm text-muted-foreground">
              {[
                wakeL && "wake L",
                draft.jets_only && "jets/turbines only",
                draft.exclude_types.length && `types ${draft.exclude_types.join(", ")}`,
              ]
                .filter(Boolean)
                .join(" · ") || "none"}
            </span>
          )}
        </div>

        {/* save / revert */}
        {canEdit && dirty && (
          <div className="flex justify-end gap-2 border-t pt-3">
            <Button
              variant="ghost"
              onClick={() => {
                setDraft(server);
                setTypesStr(server.exclude_types.join(", "));
              }}
            >
              Revert
            </Button>
            <Button
              disabled={upsert.isPending}
              onClick={() =>
                upsert.mutate({ icao: program.icao, body: draft })
              }
            >
              Save changes
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function ProgramsTab() {
  const { data: me } = useMe();
  const programs = usePrograms();
  const canEdit = hasPermission(me, "tmu.program.update");
  const canDelete = hasPermission(me, "tmu.program.delete");

  return (
    <div className="flex flex-col gap-6">
      {canEdit && <SetProgramForm />}

      {programs.isError ? (
        <p className="py-8 text-center text-sm text-muted-foreground">
          Couldn&apos;t load programs.
        </p>
      ) : !programs.data ? (
        <p className="py-8 text-center text-sm text-muted-foreground">Loading…</p>
      ) : programs.data.length === 0 ? (
        <Card>
          <CardContent className="py-10 text-center">
            <p className="text-sm text-muted-foreground">
              No TMU programs active.
              {canEdit && " Set an arrival rate for an airport above."}
            </p>
          </CardContent>
        </Card>
      ) : (
        programs.data.map((p) => (
          <ProgramCard
            key={p.icao}
            program={p}
            canEdit={canEdit}
            canDelete={canDelete}
          />
        ))
      )}
    </div>
  );
}

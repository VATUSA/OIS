import {useMemo, useState} from "react";
import {useNavigate} from "@tanstack/react-router";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  EmptyState,
  Input,
  QueryState,
  Select,
  StatusPill,
  useToast,
} from "@ois/ui";
import {Clock, Gauge, Plane, Plus, Route, X} from "lucide-react";

import {useView} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {useAirportFlow} from "@/lib/feed";
import {hasPermission} from "@/lib/permissions";
import {formatZulu, parseZulu, timeAgo} from "@/lib/time";
import {type GateRule, type Program, useDeleteProgram, usePrograms, useUpsertProgram,} from "@/lib/tmu";

const LABEL = "flex flex-col gap-1 text-xs font-semibold text-ink-2";
const SECTION = "text-xs font-semibold text-ink-2";

// Live arrival demand for the airport, polled from the VATSIM feed.
function LiveDemand({ icao, aar, compact = false }: { icao: string; aar: number; compact?: boolean }) {
  const flow = useAirportFlow(icao);
  if (!flow.data) {
    return <span className="text-xs text-ink-3">live demand…</span>;
  }
  const d = flow.data;
  const over = d.demand_60min > aar;
  const pill = (
    <StatusPill tone={over ? "bad" : "good"} className="font-mono">
      {d.demand_60min}/{aar} · 60 min
    </StatusPill>
  );
  if (compact) return pill;
  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      {pill}
      {over && <span className="font-semibold text-danger">over capacity</span>}
      <span className="text-ink-2">
        <span className="font-mono">{d.inbound}</span> inbound — <span className="font-mono">{d.airborne}</span>{" "}
        airborne · <span className="font-mono">{d.ground}</span> ground ·{" "}
        <span className="font-mono">{d.proposed}</span> proposed
      </span>
    </div>
  );
}

const TRAIL_OPTS: [number, string][] = [
  [0, "AUTO"],
  [3, "3 min"],
  [5, "5 min"],
  [7, "7 min"],
  [10, "10 min"],
  [15, "15 min"],
  [20, "20 min"],
];

type Draft = {
  aar: number;
  trail: number;
  mit: number;
  gates: GateRule[];
  exclude_wake: string[];
  exclude_types: string[];
  jets_only: boolean;
  active_until: string | null;
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
    active_until: p.active_until ?? null,
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
  if (trail > 0) return `${trail} MINIT`;
  return "AUTO";
}

function TrailSelect({
  value,
  onChange,
  disabled,
  title,
}: {
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <Select
      className="font-mono"
      value={value}
      disabled={disabled}
      title={title}
      onChange={(e) => onChange(Number(e.target.value))}
    >
      {TRAIL_OPTS.map(([v, label]) => (
        <option key={v} value={v}>
          {label}
        </option>
      ))}
    </Select>
  );
}

// --- add a new airport program (the tab's control row) ---

function SetProgramForm() {
  const upsert = useUpsertProgram();
  const toast = useToast();
  const [icao, setIcao] = useState("");
  const [aar, setAar] = useState("30");
  const [trail, setTrail] = useState(0);
  const [mit, setMit] = useState("");
  const [until, setUntil] = useState("");

  function submit() {
    const cleanIcao = icao.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (cleanIcao.length < 3 || cleanIcao.length > 4) {
      toast.warning("Enter a 3–4 character ICAO");
      return;
    }
    const aarN = Math.round(Number(aar));
    if (!Number.isFinite(aarN) || aarN < 1 || aarN > 200) {
      toast.warning("AAR must be between 1 and 200");
      return;
    }
    let activeUntil: string | null = null;
    if (until.trim()) {
      activeUntil = parseZulu(until);
      if (!activeUntil) {
        toast.warning("Active until must be DD/HHMMz (e.g. 12/0400z)");
        return;
      }
    }
    const mitN = mit.trim() ? Math.round(Number(mit)) : 0;
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
          active_until: activeUntil,
        },
      },
      {
        onSuccess: () => {
          setIcao("");
          setAar("30");
          setTrail(0);
          setMit("");
          setUntil("");
        },
      },
    );
  }

  return (
    <div className="flex flex-wrap items-end gap-3">
      <label className={LABEL}>
        Airport
        <Input
          className="w-24 font-mono uppercase"
          maxLength={4}
          placeholder="KORD"
          value={icao}
          onChange={(e) => setIcao(e.target.value)}
        />
      </label>
      <label className={LABEL}>
        AAR
        <Input
          className="w-20 font-mono"
          type="number"
          min={1}
          max={200}
          value={aar}
          onChange={(e) => setAar(e.target.value)}
        />
      </label>
      <label className={LABEL}>
        Trail (MINIT)
        <TrailSelect value={trail} onChange={setTrail} disabled={!!mit.trim()} title="Minutes in trail" />
      </label>
      <label className={LABEL}>
        MIT (nm)
        <Input
          className="w-28 font-mono"
          type="number"
          min={0}
          max={300}
          placeholder="override"
          value={mit}
          onChange={(e) => setMit(e.target.value)}
        />
      </label>
      <label className={LABEL}>
        Active until
        <Input
          className="w-28 font-mono"
          placeholder="DD/HHMMz"
          value={until}
          onChange={(e) => setUntil(e.target.value)}
        />
      </label>
      <Button disabled={upsert.isPending} onClick={submit}>
        <Plus />
        Set program
      </Button>
    </div>
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
  const untilOf = (d: Draft) => (d.active_until ? formatZulu(d.active_until) : "");
  const [untilStr, setUntilStr] = useState(untilOf(server));
  // Reset local edits when the server record changes (e.g. after a save).
  const [syncKey, setSyncKey] = useState(program.updated_at);
  if (program.updated_at !== syncKey) {
    setSyncKey(program.updated_at);
    setDraft(server);
    setTypesStr(server.exclude_types.join(", "));
    setUntilStr(untilOf(server));
  }

  const endsInPast =
    !!draft.active_until && new Date(draft.active_until).getTime() < Date.now();

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
  const readValue = "font-mono text-sm font-semibold text-ink";

  return (
    <Card className="flex flex-col gap-4 p-4">
      {/* header */}
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-mono text-xl font-bold">{program.icao}</span>
        {draft.active_until && (
          <StatusPill tone={endsInPast ? "bad" : "neutral"} className="font-mono">
            {endsInPast ? "ended" : "until"} {formatZulu(draft.active_until)}
          </StatusPill>
        )}
        <div className="ml-auto flex items-center gap-3 text-xs text-ink-3">
          {program.updated_by && (
            <span>
              {program.updated_by} · {timeAgo(program.updated_at)}
            </span>
          )}
          {canDelete && (
            <ConfirmButton
              size="sm"
              onConfirm={() => del.mutate(program.icao)}
              warn={`Remove the ${program.icao} program?`}
            >
              Remove
            </ConfirmButton>
          )}
        </div>
      </div>

      <LiveDemand icao={program.icao} aar={draft.aar} />

      {/* airport-wide spacing */}
      <div className="flex flex-wrap items-end gap-4">
        <label className={LABEL}>
          AAR
          {canEdit ? (
            <Input
              className="w-20 font-mono"
              type="number"
              min={1}
              max={200}
              value={draft.aar}
              onChange={(e) => patch({ aar: Number(e.target.value) })}
            />
          ) : (
            <span className={readValue}>{draft.aar}/hr</span>
          )}
        </label>
        <label className={LABEL}>
          Trail (MINIT)
          {canEdit ? (
            <TrailSelect
              value={draft.trail}
              disabled={draft.mit > 0}
              onChange={(v) => patch({ trail: v, mit: 0 })}
              title="Minutes in trail"
            />
          ) : (
            <span className={readValue}>{draft.trail > 0 ? `${draft.trail} MINIT` : "AUTO"}</span>
          )}
        </label>
        <label className={LABEL}>
          MIT (nm)
          {canEdit ? (
            <Input
              className="w-28 font-mono"
              type="number"
              min={0}
              max={300}
              placeholder="override"
              value={draft.mit || ""}
              onChange={(e) => patch({ mit: Math.max(0, Number(e.target.value) || 0) })}
            />
          ) : (
            <span className={readValue}>{draft.mit > 0 ? `${draft.mit} MIT` : "—"}</span>
          )}
        </label>
        <label className={LABEL}>
          Active until
          {canEdit ? (
            <Input
              className="w-28 font-mono"
              placeholder="DD/HHMMz"
              value={untilStr}
              onChange={(e) => {
                setUntilStr(e.target.value);
                patch({
                  active_until: e.target.value.trim() ? parseZulu(e.target.value) : null,
                });
              }}
            />
          ) : (
            <span className={readValue}>
              {draft.active_until ? formatZulu(draft.active_until) : "indefinite"}
            </span>
          )}
        </label>
        <span className="pb-2 text-xs text-ink-3">
          airport-wide default · blank “active until” keeps the program until you remove it
        </span>
      </div>

      {/* per-gate restrictions */}
      <div className="flex flex-col gap-2 border-t border-line-soft pt-3">
        <span className={SECTION}>Gate restrictions</span>
        {draft.gates.length === 0 && (
          <span className="text-sm text-ink-3">None — all arrivals use the airport-wide spacing.</span>
        )}
        {draft.gates.map((g, i) => (
          <div key={i} className="flex flex-wrap items-center gap-2">
            <span className="text-ink-3">↳</span>
            {canEdit ? (
              <>
                <Input
                  className="w-32 font-mono uppercase"
                  maxLength={8}
                  placeholder="JJEDI4"
                  value={g.name}
                  onChange={(e) => patchGate(i, { name: e.target.value })}
                />
                <span className="text-[10px] font-semibold text-ink-3">MINIT</span>
                <TrailSelect
                  value={g.trail ?? 0}
                  disabled={(g.mit ?? 0) > 0}
                  onChange={(v) => patchGate(i, { trail: v, mit: 0 })}
                  title="Minutes in trail"
                />
                <span className="text-[10px] font-semibold text-ink-3">MIT</span>
                <Input
                  className="w-24 font-mono"
                  type="number"
                  min={0}
                  max={300}
                  placeholder="MIT nm"
                  title="Miles in trail"
                  value={g.mit || ""}
                  onChange={(e) => patchGate(i, { mit: Math.max(0, Number(e.target.value) || 0) })}
                />
                <span className="font-mono text-xs text-ink-2">{spacingLabel(g.trail ?? 0, g.mit ?? 0)}</span>
                <Button
                  size="icon"
                  variant="ghost"
                  className="size-8 text-ink-3 hover:text-danger"
                  title="Remove gate"
                  aria-label="Remove gate"
                  onClick={() => patch({ gates: draft.gates.filter((_, gi) => gi !== i) })}
                >
                  <X />
                </Button>
              </>
            ) : (
              <span className="text-sm">
                <b className="font-mono font-semibold">{g.name}</b> —{" "}
                <span className="font-mono">{spacingLabel(g.trail ?? 0, g.mit ?? 0)}</span>
              </span>
            )}
          </div>
        ))}
        {canEdit && draft.gates.length < 10 && (
          <Button
            size="sm"
            variant="ghost"
            className="w-fit text-brand-ink"
            onClick={() => patch({ gates: [...draft.gates, { name: "", trail: 0, mit: 0 }] })}
          >
            <Plus />
            Add gate <span className="font-mono">({draft.gates.length}/10)</span>
          </Button>
        )}
      </div>

      {/* aircraft exclusions */}
      <div className="flex flex-wrap items-center gap-4 border-t border-line-soft pt-3">
        <span className={SECTION}>Exclude</span>
        {canEdit ? (
          <>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="size-3.5 accent-brand"
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
            <label className="flex items-center gap-2 text-sm text-ink-2">
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
          <span className="text-sm text-ink-2">
            {[
              wakeL && "wake L",
              draft.exclude_types.length && `types ${draft.exclude_types.join(", ")}`,
            ]
              .filter(Boolean)
              .join(" · ") || "none"}
          </span>
        )}
      </div>

      {/* aircraft inclusion — kept separate from "Exclude" above; jets_only is an inclusion
          filter (checked = keep only jets/turbines), not an exclusion (see #95). */}
      <div className="flex flex-wrap items-center gap-4 border-t border-line-soft pt-3">
        <span className={SECTION}>Limit to</span>
        {canEdit ? (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="size-3.5 accent-brand"
              checked={draft.jets_only}
              onChange={(e) => patch({ jets_only: e.target.checked })}
            />
            Jets / turbines only
          </label>
        ) : (
          <span className="text-sm text-ink-2">{draft.jets_only ? "jets/turbines only" : "none"}</span>
        )}
      </div>

      {/* save / revert */}
      {canEdit && dirty && (
        <div className="flex justify-end gap-2 border-t border-line-soft pt-3">
          <Button
            variant="ghost"
            onClick={() => {
              setDraft(server);
              setTypesStr(server.exclude_types.join(", "));
            }}
          >
            Revert
          </Button>
          <Button disabled={upsert.isPending} onClick={() => upsert.mutate({ icao: program.icao, body: draft })}>
            Save changes
          </Button>
        </div>
      )}
    </Card>
  );
}

// --- table view ---

function RemoveProgram({ icao }: { icao: string }) {
  const del = useDeleteProgram();
  return (
    <ConfirmButton size="sm" onConfirm={() => del.mutate(icao)} warn={`Remove the ${icao} program?`}>
      Remove
    </ConfirmButton>
  );
}

function ProgramsTable({ programs, canEdit, canDelete }: { programs: Program[]; canEdit: boolean; canDelete: boolean }) {
  const navigate = useNavigate();

  const columns = useMemo<DataColumn<Program>[]>(() => {
    // Editing happens on the program card, so "Edit" switches to the board view.
    const openBoard = () =>
      void navigate({
        to: "/ops/tmu",
        search: (prev) => ({ ...prev, view: "board" }),
        replace: true,
        resetScroll: false,
      });
    return [
      {
        accessorKey: "icao",
        header: "Airport",
        icon: Plane,
        mono: true,
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      {
        accessorKey: "aar",
        header: "AAR",
        icon: Gauge,
        mono: true,
        align: "right",
        cell: (c) => `${c.getValue<number>()}/hr`,
      },
      {
        id: "spacing",
        accessorFn: (p) => spacingLabel(p.trail, p.mit),
        header: "Spacing",
        icon: Route,
        mono: true,
      },
      {
        id: "gates",
        accessorFn: (p) => p.gates.length,
        header: "Gates",
        mono: true,
        cell: (c) => {
          const gates = c.row.original.gates;
          return gates.length === 0 ? (
            <span className="text-ink-3">—</span>
          ) : (
            <span className="whitespace-nowrap">
              {gates.map((g) => `${g.name} ${spacingLabel(g.trail ?? 0, g.mit ?? 0)}`).join(" · ")}
            </span>
          );
        },
      },
      {
        id: "filters",
        header: "Exclude / limit",
        enableSorting: false,
        cell: (c) => {
          const p = c.row.original;
          const parts = [
            p.exclude_wake.includes("L") && "wake L",
            p.exclude_types.length > 0 && `types ${p.exclude_types.join(", ")}`,
            p.jets_only && "jets only",
          ].filter(Boolean);
          return <span className="text-ink-2">{parts.join(" · ") || "—"}</span>;
        },
      },
      {
        id: "demand",
        header: "Live demand",
        enableSorting: false,
        cell: (c) => <LiveDemand icao={c.row.original.icao} aar={c.row.original.aar} compact />,
      },
      {
        accessorKey: "active_until",
        header: "Until",
        icon: Clock,
        mono: true,
        cell: (c) => {
          const until = c.getValue<string | null>();
          if (!until) return <span className="text-ink-3">indefinite</span>;
          const past = new Date(until).getTime() < Date.now();
          return <span className={past ? "text-danger" : undefined}>{formatZulu(until)}</span>;
        },
      },
      {
        accessorKey: "updated_at",
        header: "Updated",
        cell: (c) => (
          <span className="whitespace-nowrap text-xs text-ink-3">
            {c.row.original.updated_by ? `${c.row.original.updated_by} · ` : ""}
            {timeAgo(c.getValue<string>())}
          </span>
        ),
      },
      {
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => (
          <div className="flex justify-end gap-1">
            {canEdit && (
              <Button size="sm" variant="ghost" onClick={openBoard}>
                Edit
              </Button>
            )}
            {canDelete && <RemoveProgram icao={c.row.original.icao} />}
          </div>
        ),
      },
    ];
  }, [navigate, canEdit, canDelete]);

  return (
    <DataTable
      label="TMU programs"
      columns={columns}
      data={programs}
      getRowId={(p) => p.icao}
      initialSort={[{ id: "icao", desc: false }]}
      rowCap={25}
    />
  );
}

export function ProgramsTab() {
  const { data: me } = useMe();
  const programs = usePrograms();
  const view = useView();
  const canEdit = hasPermission(me, "tmu.program.update");
  const canDelete = hasPermission(me, "tmu.program.delete");

  return (
    <div className="flex flex-col gap-6">
      {canEdit && <SetProgramForm />}

      <QueryState
        isLoading={!programs.data}
        isError={programs.isError}
        onRetry={() => programs.refetch()}
        error="Couldn't load programs."
      >
        {programs.data?.length === 0 ? (
          <EmptyState icon={Gauge} className="rounded-md border border-line">
            No TMU programs active.
            {canEdit && " Set an arrival rate for an airport above."}
          </EmptyState>
        ) : view === "table" ? (
          <ProgramsTable programs={programs.data ?? []} canEdit={canEdit} canDelete={canDelete} />
        ) : (
          <div className="flex flex-col gap-4">
            {(programs.data ?? []).map((p) => (
              <ProgramCard key={p.icao} program={p} canEdit={canEdit} canDelete={canDelete} />
            ))}
          </div>
        )}
      </QueryState>
    </div>
  );
}

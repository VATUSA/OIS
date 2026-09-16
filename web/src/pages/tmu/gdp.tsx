import {useMemo, useState} from "react";
import {
  Bars,
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  Input,
  MetricCard,
  QueryState,
  StatusPill,
  useToast,
} from "@ois/ui";
import {CircleDot, Clock, Gauge, Pencil, Plane, Plus, Radar, Timer} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {formatZulu, hhmmZulu} from "@/lib/time";
import {
  type AarStep,
  type CreateGdp,
  type Gdp,
  type GdpBoard,
  type GdpFlightView,
  type UpdateGdp,
  useCancelGdp,
  useCompressGdp,
  useCreateGdp,
  useDeleteGdp,
  useGdpBoard,
  useGdps,
  useLockSlot,
  usePublishGdp,
  useReviseGdp,
  useUnlockSlot,
} from "@/lib/gdp";

/** A demand bin's load level → its chart colour token. */
const LEVEL_COLOR: Record<string, string> = {
  green: "level-ok",
  yellow: "level-watch",
  red: "level-over",
};

const LABEL = "flex flex-col gap-1 text-xs font-semibold text-ink-2";
const SECTION = "mb-2 text-xs font-semibold text-ink-2";

const EMPTY: CreateGdp = {
  airport: "",
  aar: 30,
  start_time: "",
  end_time: "",
  scope: "",
  max_enroute_min: undefined,
  exempt_airborne: true,
  aar_steps: [],
};

/** Add/remove time-varying AAR steps ("@1600z 45/hr"). */
function StepsEditor({
  steps,
  onChange,
}: {
  steps: AarStep[];
  onChange: (s: AarStep[]) => void;
}) {
  const [time, setTime] = useState("");
  const [rate, setRate] = useState("");
  const add = () => {
    if (!/^\d{3,4}$/.test(time) || !Number(rate)) return;
    const hhmm = time.padStart(4, "0");
    onChange(
      [...steps.filter((s) => s.start_time !== hhmm), { start_time: hhmm, aar: Number(rate) }].sort(
        (a, b) => a.start_time.localeCompare(b.start_time),
      ),
    );
    setTime("");
    setRate("");
  };
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-semibold text-ink-2">Rate steps (optional)</span>
      <div className="flex flex-wrap items-center gap-1.5">
        {steps.map((s) => (
          <span
            key={s.start_time}
            className="flex items-center gap-1 rounded-full border border-line bg-chip py-0.5 pl-2.5 pr-1.5 font-mono text-xs"
          >
            @{s.start_time}z {s.aar}/hr
            <button
              type="button"
              aria-label="Remove step"
              onClick={() => onChange(steps.filter((x) => x.start_time !== s.start_time))}
              className="text-ink-3 hover:text-danger"
            >
              ×
            </button>
          </span>
        ))}
        <Input
          className="h-8 w-16 font-mono text-xs"
          placeholder="HHMM"
          maxLength={4}
          value={time}
          onChange={(e) => setTime(e.target.value)}
        />
        <Input
          className="h-8 w-14 font-mono text-xs"
          placeholder="AAR"
          inputMode="numeric"
          value={rate}
          onChange={(e) => setRate(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <Button size="sm" variant="outline" onClick={add} disabled={!time.trim() || !rate.trim()}>
          <Plus />
          step
        </Button>
      </div>
    </div>
  );
}

function CreateForm({ onCreated }: { onCreated: (id: string) => void }) {
  const create = useCreateGdp();
  const toast = useToast();
  const [form, setForm] = useState<CreateGdp>(EMPTY);

  function submit() {
    const airport = (form.airport ?? "").replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (airport.length < 3 || airport.length > 4) {
      toast.warning("Enter a 3–4 character airport ICAO");
      return;
    }
    if (!form.aar || form.aar < 1 || form.aar > 200) {
      toast.warning("AAR must be between 1 and 200");
      return;
    }
    if (!/^\d{3,4}$/.test(form.start_time) || !/^\d{3,4}$/.test(form.end_time)) {
      toast.warning("Enter start/end as Zulu HHMM (e.g. 1800)");
      return;
    }
    create.mutate(
      {
        airport,
        aar: Number(form.aar),
        start_time: form.start_time,
        end_time: form.end_time,
        scope: form.scope,
        max_enroute_min: form.max_enroute_min ? Number(form.max_enroute_min) : undefined,
        exempt_airborne: form.exempt_airborne,
        aar_steps: form.aar_steps,
      },
      {
        onSuccess: (g) => {
          setForm(EMPTY);
          onCreated(g.id);
        },
      },
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-end gap-3">
        <label className={LABEL}>
          Airport
          <Input
            className="w-24 font-mono uppercase"
            maxLength={4}
            placeholder="KSFO"
            value={form.airport}
            onChange={(e) => setForm((f) => ({ ...f, airport: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          AAR /hr
          <Input
            className="w-20 font-mono"
            inputMode="numeric"
            value={form.aar ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, aar: Number(e.target.value) || 0 }))}
          />
        </label>
        <label className={LABEL}>
          Start (Z)
          <Input
            className="w-20 font-mono"
            placeholder="1800"
            maxLength={4}
            value={form.start_time}
            onChange={(e) => setForm((f) => ({ ...f, start_time: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          End (Z)
          <Input
            className="w-20 font-mono"
            placeholder="2000"
            maxLength={4}
            value={form.end_time}
            onChange={(e) => setForm((f) => ({ ...f, end_time: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          Scope (ARTCC)
          <Input
            className="w-32 font-mono uppercase"
            placeholder="all"
            value={form.scope ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, scope: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          Max enroute (min)
          <Input
            className="w-28 font-mono"
            inputMode="numeric"
            placeholder="none"
            value={form.max_enroute_min ?? ""}
            onChange={(e) =>
              setForm((f) => ({
                ...f,
                max_enroute_min: e.target.value ? Number(e.target.value) : undefined,
              }))
            }
          />
        </label>
        <label className="flex items-center gap-2 pb-2 text-sm">
          <input
            type="checkbox"
            className="size-3.5 accent-brand"
            checked={form.exempt_airborne}
            onChange={(e) => setForm((f) => ({ ...f, exempt_airborne: e.target.checked }))}
          />
          Exempt airborne
        </label>
        <Button className="whitespace-nowrap" disabled={create.isPending} onClick={submit}>
          <Plus />
          Create GDP
        </Button>
      </div>
      <StepsEditor
        steps={form.aar_steps ?? []}
        onChange={(s) => setForm((f) => ({ ...f, aar_steps: s }))}
      />
      <p className="text-xs text-ink-3">
        Meters inbound demand to a constrained airport down to its AAR, assigning frozen EDCTs to
        not-yet-departed flights via Ration-By-Schedule. Airborne traffic is exempt.
      </p>
    </div>
  );
}

function GdpActions({ gdp, canPublish, canDelete }: { gdp: Gdp; canPublish: boolean; canDelete: boolean }) {
  const cancel = useCancelGdp();
  const del = useDeleteGdp();

  return (
    <div className="flex justify-end gap-1" onClick={(e) => e.stopPropagation()}>
      {canPublish && (gdp.status === "draft" || gdp.status === "published") && (
        <Button size="sm" variant="ghost" disabled={cancel.isPending} onClick={() => cancel.mutate(gdp.id)}>
          Cancel
        </Button>
      )}
      {canDelete && (
        <ConfirmButton size="sm" onConfirm={() => del.mutate(gdp.id)} warn={`Delete the GDP for ${gdp.airport}?`}>
          Delete
        </ConfirmButton>
      )}
    </div>
  );
}

type DemandBin = GdpBoard["demand"][number];
const binLabel = (b: DemandBin) => hhmmZulu(b.start);
const binCount = (b: DemandBin) => b.count;
const binColor = (b: DemandBin) => LEVEL_COLOR[b.level] ?? "ink-3";

function DemandChart({ demand }: { demand: GdpBoard["demand"] }) {
  // The chart draws one capacity rule; with AAR steps the per-bin cap varies, so the rule is shown
  // only while it's uniform (each bar's level colour still reflects its own cap).
  const caps = new Set(demand.map((b) => b.cap));
  const cap = caps.size === 1 ? demand[0]?.cap : undefined;
  return (
    <Bars
      data={demand}
      category={binLabel}
      value={binCount}
      color={binColor}
      cap={cap}
      height={160}
      label="Demand vs AAR"
    />
  );
}

function FlightsTable({
  rows,
  published,
  exempt,
  gdpId,
  canPublish,
}: {
  rows: GdpFlightView[];
  published: boolean;
  exempt?: boolean;
  gdpId?: string;
  canPublish?: boolean;
}) {
  const lock = useLockSlot();
  const unlock = useUnlockSlot();
  // Lock/unlock only make sense for the controlled table of a published program.
  const showActions = !exempt && published && !!canPublish && !!gdpId;

  const columns = useMemo<DataColumn<GdpFlightView>[]>(() => {
    const cols: DataColumn<GdpFlightView>[] = [
      {
        accessorKey: "cs",
        header: "Callsign",
        icon: Plane,
        mono: true,
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      { accessorKey: "dep", header: "From", mono: true, cell: (c) => c.getValue<string>() || "—" },
      { accessorKey: "eta", header: "ETA", icon: Clock, mono: true, cell: (c) => `${hhmmZulu(c.getValue<string>())}z` },
    ];
    if (exempt) {
      cols.push({
        accessorKey: "exempt_reason",
        header: "Reason",
        cell: (c) => <span className="text-xs text-ink-2">{c.getValue<string>() ?? "—"}</span>,
      });
    } else {
      cols.push(
        { accessorKey: "cta", header: "CTA", mono: true, cell: (c) => `${hhmmZulu(c.getValue<string>())}z` },
        {
          accessorKey: "edct",
          header: "EDCT",
          icon: Timer,
          mono: true,
          cell: (c) => (c.getValue<string>() ? `${hhmmZulu(c.getValue<string>())}z` : "—"),
        },
        {
          accessorKey: "delay_min",
          header: "Delay",
          mono: true,
          align: "right",
          cell: (c) =>
            c.getValue<number>() > 0 ? (
              <span className="text-warning">+{c.getValue<number>()}m</span>
            ) : (
              <span className="font-sans text-ink-3">on time</span>
            ),
        },
      );
    }
    cols.push({
      accessorKey: "status",
      header: "Status",
      icon: CircleDot,
      cell: (c) => {
        const f = c.row.original;
        const popup = published && !exempt && !f.frozen;
        return (
          <span className="flex items-center gap-2 whitespace-nowrap text-xs">
            <span className="text-ink-2">{f.status}</span>
            {f.frozen && <StatusPill tone="good">frozen</StatusPill>}
            {popup && <StatusPill tone="neutral">pop-up</StatusPill>}
          </span>
        );
      },
    });
    if (showActions) {
      cols.push({
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => {
          const f = c.row.original;
          return f.frozen ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={unlock.isPending}
              onClick={() => unlock.mutate({ id: gdpId!, callsign: f.cs })}
            >
              Unlock
            </Button>
          ) : (
            <Button
              size="sm"
              variant="secondary"
              disabled={lock.isPending}
              onClick={() => lock.mutate({ id: gdpId!, callsign: f.cs })}
            >
              Lock
            </Button>
          );
        },
      });
    }
    return cols;
  }, [exempt, published, showActions, gdpId, lock, unlock]);

  return (
    <DataTable
      label={exempt ? "Exempt inbounds" : "Controlled flights"}
      columns={columns}
      data={rows}
      getRowId={(f) => f.cs}
      rowCap={25}
      empty={exempt ? "No exempt inbounds." : "No controlled flights."}
    />
  );
}

/** Inline edit form for a GDP's parameters (airport is immutable). */
function ReviseForm({ board, onDone }: { board: GdpBoard; onDone: () => void }) {
  const revise = useReviseGdp();
  const toast = useToast();
  const [form, setForm] = useState<UpdateGdp>({
    aar: board.aar,
    start_time: board.start_time,
    end_time: board.end_time,
    scope: board.scope,
    max_enroute_min: board.max_enroute_min ?? undefined,
    exempt_airborne: board.exempt_airborne,
    aar_steps: board.aar_steps,
  });

  function submit() {
    if (!form.aar || form.aar < 1 || form.aar > 200) {
      toast.warning("AAR must be between 1 and 200");
      return;
    }
    if (!/^\d{3,4}$/.test(form.start_time) || !/^\d{3,4}$/.test(form.end_time)) {
      toast.warning("Enter start/end as Zulu HHMM (e.g. 1800)");
      return;
    }
    revise.mutate(
      {
        id: board.id,
        body: {
          aar: Number(form.aar),
          start_time: form.start_time,
          end_time: form.end_time,
          scope: form.scope,
          max_enroute_min: form.max_enroute_min ? Number(form.max_enroute_min) : null,
          exempt_airborne: form.exempt_airborne,
          aar_steps: form.aar_steps,
        },
      },
      { onSuccess: onDone },
    );
  }

  return (
    <div className="flex flex-col gap-3 rounded-sm border border-line bg-panel-2 p-3">
      <div className="flex items-center gap-2 text-sm font-semibold">
        <Pencil className="size-3.5 text-ink-3" />
        Revise <span className="font-mono">{board.airport}</span>
      </div>
      {board.status === "published" && (
        <p className="rounded-xs bg-warning-soft px-2 py-1 text-xs text-warning">
          This program is live — saving re-rations off the current feed and reissues EDCTs to
          controlled flights.
        </p>
      )}
      <div className="flex flex-wrap items-end gap-3">
        <label className={LABEL}>
          AAR /hr
          <Input
            className="w-20 font-mono"
            inputMode="numeric"
            value={form.aar ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, aar: Number(e.target.value) || 0 }))}
          />
        </label>
        <label className={LABEL}>
          Start (Z)
          <Input
            className="w-20 font-mono"
            maxLength={4}
            value={form.start_time}
            onChange={(e) => setForm((f) => ({ ...f, start_time: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          End (Z)
          <Input
            className="w-20 font-mono"
            maxLength={4}
            value={form.end_time}
            onChange={(e) => setForm((f) => ({ ...f, end_time: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          Scope (ARTCC)
          <Input
            className="w-32 font-mono uppercase"
            placeholder="all"
            value={form.scope ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, scope: e.target.value }))}
          />
        </label>
        <label className={LABEL}>
          Max enroute (min)
          <Input
            className="w-28 font-mono"
            inputMode="numeric"
            placeholder="none"
            value={form.max_enroute_min ?? ""}
            onChange={(e) =>
              setForm((f) => ({
                ...f,
                max_enroute_min: e.target.value ? Number(e.target.value) : undefined,
              }))
            }
          />
        </label>
        <label className="flex items-center gap-2 pb-2 text-sm">
          <input
            type="checkbox"
            className="size-3.5 accent-brand"
            checked={form.exempt_airborne}
            onChange={(e) => setForm((f) => ({ ...f, exempt_airborne: e.target.checked }))}
          />
          Exempt airborne
        </label>
      </div>
      <StepsEditor
        steps={form.aar_steps ?? []}
        onChange={(s) => setForm((f) => ({ ...f, aar_steps: s }))}
      />
      <div className="flex gap-2">
        <Button disabled={revise.isPending} onClick={submit}>
          Save changes
        </Button>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

function BoardView({
  id,
  canPublish,
  canRevise,
}: {
  id: string;
  canPublish: boolean;
  canRevise: boolean;
}) {
  const board = useGdpBoard(id);
  const publish = usePublishGdp();
  const compress = useCompressGdp();
  const [editing, setEditing] = useState(false);
  const b = board.data;

  if (!b) {
    return (
      <Card>
        <QueryState
          isLoading
          isError={board.isError}
          onRetry={() => board.refetch()}
          error="Couldn't load the GDP board."
        />
      </Card>
    );
  }

  return (
    <Card className="flex flex-col gap-5 p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="font-mono text-xl font-bold">{b.airport}</h2>
          <StatusPill tone={toneOf("publish", b.status)}>{b.status}</StatusPill>
          <span className="font-mono text-xs text-ink-2">
            AAR {b.aar}/hr
            {b.aar_steps.map((s) => ` → ${s.aar} @${s.start_time}z`).join("")} ·{" "}
            {hhmmZulu(b.window_start)}z–{hhmmZulu(b.window_end)}z
            {b.scope ? ` · ${b.scope}` : ""}
            {b.max_enroute_min ? ` · ≤${b.max_enroute_min}m` : ""}
          </span>
        </div>
        <div className="flex gap-2">
          {canRevise && (b.status === "draft" || b.status === "published") && !editing && (
            <Button variant="outline" onClick={() => setEditing(true)}>
              <Pencil />
              Revise
            </Button>
          )}
          {canPublish && b.status === "draft" && (
            <Button disabled={publish.isPending} onClick={() => publish.mutate(b.id)}>
              Publish &amp; freeze EDCTs
            </Button>
          )}
          {canPublish && b.status === "published" && (
            <Button
              variant="secondary"
              disabled={compress.isPending}
              onClick={() => compress.mutate(b.id)}
              title="Reclaim capacity freed by departed/cancelled flights — pulls EDCTs earlier"
            >
              Compress
            </Button>
          )}
        </div>
      </div>

      {editing && <ReviseForm key={b.id} board={b} onDone={() => setEditing(false)} />}

      {b.status === "draft" && !editing && (
        <p className="rounded-xs bg-panel-2 px-3 py-2 text-xs text-ink-2">
          Draft preview — control times are advisory and recompute live. Publish to freeze EDCTs so
          they hold.
        </p>
      )}

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        <MetricCard label="Controlled" icon={Plane} value={b.stats.controlled} />
        <MetricCard label="Exempt" value={b.stats.exempt} />
        <MetricCard label="Avg delay" icon={Timer} value={`${b.stats.avg_delay_min}m`} />
        <MetricCard label="Max delay" value={`${b.stats.max_delay_min}m`} />
        <MetricCard label="Total delay" value={`${b.stats.total_delay_min}m`} />
      </div>

      <div>
        <p className={SECTION}>
          Demand vs AAR (<span className="font-mono">15</span>-min bins)
        </p>
        <DemandChart demand={b.demand} />
      </div>

      <div>
        <p className={SECTION}>Controlled flights</p>
        <FlightsTable rows={b.flights} published={b.published} gdpId={b.id} canPublish={canPublish} />
      </div>

      {b.exempt.length > 0 && (
        <div>
          <p className={SECTION}>Exempt inbounds</p>
          <FlightsTable rows={b.exempt} published={b.published} exempt />
        </div>
      )}
    </Card>
  );
}

export function GdpTab() {
  const { data: me } = useMe();
  const gdps = useGdps();
  const canCreate = hasPermission(me, "tmu.gdp.create");
  const canPublish = hasPermission(me, "tmu.gdp.publish");
  const canDelete = hasPermission(me, "tmu.gdp.delete");
  const [selected, setSelected] = useState<string | null>(null);

  const columns = useMemo<DataColumn<Gdp>[]>(
    () => [
      {
        accessorKey: "status",
        header: "Status",
        icon: CircleDot,
        cell: (c) => <StatusPill tone={toneOf("publish", c.getValue<string>())}>{c.getValue<string>()}</StatusPill>,
      },
      {
        accessorKey: "airport",
        header: "Airport",
        icon: Plane,
        mono: true,
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      { accessorKey: "aar", header: "AAR", icon: Gauge, mono: true, align: "right", cell: (c) => `${c.getValue<number>()}/hr` },
      {
        id: "window",
        accessorFn: (g) => g.start_time,
        header: "Window",
        icon: Clock,
        mono: true,
        cell: (c) => (
          <span className="whitespace-nowrap">
            {c.row.original.start_time}z–{c.row.original.end_time}z
          </span>
        ),
      },
      {
        accessorKey: "scope",
        header: "Scope",
        icon: Radar,
        mono: true,
        cell: (c) => c.getValue<string>() || <span className="text-ink-3">All</span>,
      },
      {
        accessorKey: "max_enroute_min",
        header: "Tier",
        mono: true,
        cell: (c) => (
          <span className="text-ink-2">{c.getValue<number>() ? `≤${c.getValue<number>()}m` : "—"}</span>
        ),
      },
      {
        accessorKey: "updated_at",
        header: "Updated",
        mono: true,
        cell: (c) => (
          <span className="whitespace-nowrap text-ink-3">
            {formatZulu(c.getValue<string>())}
            {c.row.original.updated_by ? ` · ${c.row.original.updated_by}` : ""}
          </span>
        ),
      },
      {
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => <GdpActions gdp={c.row.original} canPublish={canPublish} canDelete={canDelete} />,
      },
    ],
    [canPublish, canDelete],
  );

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm onCreated={setSelected} />}

      <DataTable
        label="Ground delay programs"
        columns={columns}
        data={gdps.data ?? []}
        getRowId={(g) => g.id}
        rowCap={25}
        // Clicking the selected row keeps it open (the board only changes on another row).
        selection={{ mode: "single", selected, onChange: (id) => id && setSelected(id) }}
        isLoading={!gdps.data}
        isError={gdps.isError}
        onRetry={() => gdps.refetch()}
        empty="No ground delay programs."
      />

      {selected && <BoardView key={selected} id={selected} canPublish={canPublish} canRevise={canCreate} />}
    </div>
  );
}

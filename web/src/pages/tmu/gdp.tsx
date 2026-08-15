import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input, useToast} from "@ois/ui";
import {Pencil, Plus} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {formatZulu, hhmmZulu} from "@/lib/time";
import {
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

function statusVariant(
  status: string,
): "secondary" | "success" | "destructive" | "outline" {
  if (status === "published") return "success";
  if (status === "cancelled") return "destructive";
  if (status === "expired") return "outline";
  return "secondary";
}

const LEVEL_BG: Record<string, string> = {
  green: "bg-emerald-500/70",
  yellow: "bg-amber-500/80",
  red: "bg-red-500/80",
};

const EMPTY: CreateGdp = {
  airport: "",
  aar: 30,
  start_time: "",
  end_time: "",
  scope: "",
  max_enroute_min: undefined,
  exempt_airborne: true,
};

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
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <p className="text-sm text-muted-foreground">
          Meters inbound demand to a constrained airport down to its AAR, assigning frozen
          EDCTs to not-yet-departed flights via Ration-By-Schedule. Airborne traffic is exempt.
        </p>
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Airport
            </span>
            <Input
              className="w-24 font-mono uppercase"
              maxLength={4}
              placeholder="KSFO"
              value={form.airport}
              onChange={(e) => setForm((f) => ({ ...f, airport: e.target.value }))}
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              AAR /hr
            </span>
            <Input
              className="w-20 font-mono"
              inputMode="numeric"
              value={form.aar ?? ""}
              onChange={(e) =>
                setForm((f) => ({ ...f, aar: Number(e.target.value) || 0 }))
              }
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Start (Z)
            </span>
            <Input
              className="w-20 font-mono"
              placeholder="1800"
              maxLength={4}
              value={form.start_time}
              onChange={(e) => setForm((f) => ({ ...f, start_time: e.target.value }))}
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              End (Z)
            </span>
            <Input
              className="w-20 font-mono"
              placeholder="2000"
              maxLength={4}
              value={form.end_time}
              onChange={(e) => setForm((f) => ({ ...f, end_time: e.target.value }))}
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Scope (ARTCC)
            </span>
            <Input
              className="w-32 font-mono uppercase"
              placeholder="all"
              value={form.scope ?? ""}
              onChange={(e) => setForm((f) => ({ ...f, scope: e.target.value }))}
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Max enroute (min)
            </span>
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
              className="size-4"
              checked={form.exempt_airborne}
              onChange={(e) =>
                setForm((f) => ({ ...f, exempt_airborne: e.target.checked }))
              }
            />
            Exempt airborne
          </label>
          <Button
            className="whitespace-nowrap"
            disabled={create.isPending}
            onClick={submit}
          >
            <Plus />
            Create GDP
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function GdpRow({
  gdp,
  selected,
  onSelect,
  canPublish,
  canDelete,
}: {
  gdp: Gdp;
  selected: boolean;
  onSelect: () => void;
  canPublish: boolean;
  canDelete: boolean;
}) {
  const cancel = useCancelGdp();
  const del = useDeleteGdp();

  return (
    <tr
      className={"cursor-pointer border-t " + (selected ? "bg-muted/50" : "hover:bg-muted/30")}
      onClick={onSelect}
    >
      <td className="py-2 pr-3">
        <Badge variant={statusVariant(gdp.status)}>{gdp.status}</Badge>
      </td>
      <td className="py-2 pr-3 font-mono font-medium">{gdp.airport}</td>
      <td className="py-2 pr-3 font-mono text-xs">{gdp.aar}/hr</td>
      <td className="py-2 pr-3 font-mono text-xs">
        {gdp.start_time}z–{gdp.end_time}z
      </td>
      <td className="py-2 pr-3 font-mono text-xs">
        {gdp.scope ? gdp.scope : <span className="text-muted-foreground">All</span>}
      </td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {gdp.max_enroute_min ? `≤${gdp.max_enroute_min}m` : "—"}
      </td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(gdp.updated_at)}
        {gdp.updated_by ? ` · ${gdp.updated_by}` : ""}
      </td>
      <td className="py-2 text-right" onClick={(e) => e.stopPropagation()}>
        <div className="flex justify-end gap-1">
          {canPublish && (gdp.status === "draft" || gdp.status === "published") && (
            <Button
              size="sm"
              variant="ghost"
              disabled={cancel.isPending}
              onClick={() => cancel.mutate(gdp.id)}
            >
              Cancel
            </Button>
          )}
          {canDelete && (
            <ConfirmButton
              size="sm"
              onConfirm={() => del.mutate(gdp.id)}
              warn={`Delete the GDP for ${gdp.airport}?`}
            >
              Delete
            </ConfirmButton>
          )}
        </div>
      </td>
    </tr>
  );
}

function DemandChart({ demand }: { demand: GdpBoard["demand"] }) {
  const max = Math.max(1, ...demand.map((b) => Math.max(b.count, b.cap)));
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-end gap-1" style={{ height: 96 }}>
        {demand.map((b, i) => (
          <div
            key={i}
            className="relative flex flex-1 items-end"
            title={`${hhmmZulu(b.start)}z · ${b.count}/${b.cap}`}
          >
            {/* capacity line */}
            <div
              className="absolute left-0 right-0 border-t border-dashed border-muted-foreground/50"
              style={{ bottom: `${(b.cap / max) * 100}%` }}
            />
            <div
              className={"w-full rounded-t " + (LEVEL_BG[b.level] ?? "bg-muted")}
              style={{ height: `${(b.count / max) * 100}%`, minHeight: b.count ? 2 : 0 }}
            />
          </div>
        ))}
      </div>
      <div className="flex gap-1 text-[10px] text-muted-foreground">
        {demand.map((b, i) => (
          <span key={i} className="flex-1 text-center font-mono">
            {i % 2 === 0 ? `${hhmmZulu(b.start)}` : ""}
          </span>
        ))}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="flex flex-col">
      <span className="text-lg font-semibold tabular-nums">{value}</span>
      <span className="text-xs text-muted-foreground">{label}</span>
    </div>
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

  if (rows.length === 0) {
    return (
      <p className="py-6 text-center text-sm text-muted-foreground">
        {exempt ? "No exempt inbounds." : "No controlled flights."}
      </p>
    );
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
            <th className="pb-2 pr-3 font-medium">Callsign</th>
            <th className="pb-2 pr-3 font-medium">From</th>
            <th className="pb-2 pr-3 font-medium">ETA</th>
            {exempt ? (
              <th className="pb-2 pr-3 font-medium">Reason</th>
            ) : (
              <>
                <th className="pb-2 pr-3 font-medium">CTA</th>
                <th className="pb-2 pr-3 font-medium">EDCT</th>
                <th className="pb-2 pr-3 font-medium">Delay</th>
              </>
            )}
            <th className="pb-2 pr-3 font-medium">Status</th>
            {showActions && <th className="pb-2" />}
          </tr>
        </thead>
        <tbody>
          {rows.map((f) => {
            const popup = published && !exempt && !f.frozen;
            return (
              <tr key={f.cs} className="border-t">
                <td className="py-1.5 pr-3 font-mono font-medium">{f.cs}</td>
                <td className="py-1.5 pr-3 font-mono text-xs">{f.dep || "—"}</td>
                <td className="py-1.5 pr-3 font-mono text-xs">{hhmmZulu(f.eta)}z</td>
                {exempt ? (
                  <td className="py-1.5 pr-3 text-xs text-muted-foreground">
                    {f.exempt_reason ?? "—"}
                  </td>
                ) : (
                  <>
                    <td className="py-1.5 pr-3 font-mono text-xs">{hhmmZulu(f.cta)}z</td>
                    <td className="py-1.5 pr-3 font-mono text-xs">
                      {f.edct ? `${hhmmZulu(f.edct)}z` : "—"}
                    </td>
                    <td className="py-1.5 pr-3 font-mono text-xs">
                      {f.delay_min > 0 ? (
                        <span className="text-amber-600 dark:text-amber-400">
                          +{f.delay_min}m
                        </span>
                      ) : (
                        <span className="text-muted-foreground">on time</span>
                      )}
                    </td>
                  </>
                )}
                <td className="py-1.5 pr-3 text-xs">
                  <span className="text-muted-foreground">{f.status}</span>
                  {f.frozen && (
                    <Badge variant="success" className="ml-2">
                      frozen
                    </Badge>
                  )}
                  {popup && (
                    <Badge variant="outline" className="ml-2">
                      pop-up
                    </Badge>
                  )}
                </td>
                {showActions && (
                  <td className="py-1.5 text-right">
                    {f.frozen ? (
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
                    )}
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
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
        },
      },
      { onSuccess: onDone },
    );
  }

  const field = "flex flex-col gap-1";
  const lbl =
    "text-xs font-medium uppercase tracking-wide text-muted-foreground";
  return (
    <div className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3">
      <div className="flex items-center gap-2 text-sm font-medium">
        <Pencil className="size-3.5" />
        Revise {board.airport}
      </div>
      {board.status === "published" && (
        <p className="rounded bg-amber-500/10 px-2 py-1 text-xs text-amber-700 dark:text-amber-300">
          This program is live — saving re-rations off the current feed and
          reissues EDCTs to controlled flights.
        </p>
      )}
      <div className="flex flex-wrap items-end gap-3">
        <label className={field}>
          <span className={lbl}>AAR /hr</span>
          <Input
            className="w-20 font-mono"
            inputMode="numeric"
            value={form.aar ?? ""}
            onChange={(e) =>
              setForm((f) => ({ ...f, aar: Number(e.target.value) || 0 }))
            }
          />
        </label>
        <label className={field}>
          <span className={lbl}>Start (Z)</span>
          <Input
            className="w-20 font-mono"
            maxLength={4}
            value={form.start_time}
            onChange={(e) => setForm((f) => ({ ...f, start_time: e.target.value }))}
          />
        </label>
        <label className={field}>
          <span className={lbl}>End (Z)</span>
          <Input
            className="w-20 font-mono"
            maxLength={4}
            value={form.end_time}
            onChange={(e) => setForm((f) => ({ ...f, end_time: e.target.value }))}
          />
        </label>
        <label className={field}>
          <span className={lbl}>Scope (ARTCC)</span>
          <Input
            className="w-32 font-mono uppercase"
            placeholder="all"
            value={form.scope ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, scope: e.target.value }))}
          />
        </label>
        <label className={field}>
          <span className={lbl}>Max enroute (min)</span>
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
            className="size-4"
            checked={form.exempt_airborne}
            onChange={(e) =>
              setForm((f) => ({ ...f, exempt_airborne: e.target.checked }))
            }
          />
          Exempt airborne
        </label>
        <div className="flex gap-2 pb-0.5">
          <Button disabled={revise.isPending} onClick={submit}>
            Save changes
          </Button>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
        </div>
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

  if (board.isError) {
    return (
      <Card>
        <CardContent className="py-8 text-center text-sm text-muted-foreground">
          Couldn&apos;t load the GDP board.
        </CardContent>
      </Card>
    );
  }
  if (!b) {
    return (
      <Card>
        <CardContent className="py-8 text-center text-sm text-muted-foreground">
          Loading…
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-5 pt-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <h3 className="font-mono text-lg font-semibold">{b.airport}</h3>
            <Badge variant={statusVariant(b.status)}>{b.status}</Badge>
            <span className="font-mono text-sm text-muted-foreground">
              AAR {b.aar}/hr · {hhmmZulu(b.window_start)}z–{hhmmZulu(b.window_end)}z
              {b.scope ? ` · ${b.scope}` : ""}
              {b.max_enroute_min ? ` · ≤${b.max_enroute_min}m` : ""}
            </span>
          </div>
          <div className="flex gap-2">
            {canRevise &&
              (b.status === "draft" || b.status === "published") &&
              !editing && (
                <Button variant="outline" onClick={() => setEditing(true)}>
                  <Pencil />
                  Revise
                </Button>
              )}
            {canPublish && b.status === "draft" && (
              <Button
                disabled={publish.isPending}
                onClick={() => publish.mutate(b.id)}
              >
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

        {editing && (
          <ReviseForm key={b.id} board={b} onDone={() => setEditing(false)} />
        )}

        {b.status === "draft" && !editing && (
          <p className="rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
            Draft preview — control times are advisory and recompute live. Publish to freeze
            EDCTs so they hold.
          </p>
        )}

        <div className="flex flex-wrap gap-8">
          <Stat label="controlled" value={b.stats.controlled} />
          <Stat label="exempt" value={b.stats.exempt} />
          <Stat label="avg delay" value={`${b.stats.avg_delay_min}m`} />
          <Stat label="max delay" value={`${b.stats.max_delay_min}m`} />
          <Stat label="total delay" value={`${b.stats.total_delay_min}m`} />
        </div>

        <div>
          <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Demand vs AAR ({15}-min bins)
          </p>
          <DemandChart demand={b.demand} />
        </div>

        <div>
          <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Controlled flights
          </p>
          <FlightsTable
            rows={b.flights}
            published={b.published}
            gdpId={b.id}
            canPublish={canPublish}
          />
        </div>

        {b.exempt.length > 0 && (
          <div>
            <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Exempt inbounds
            </p>
            <FlightsTable rows={b.exempt} published={b.published} exempt />
          </div>
        )}
      </CardContent>
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

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm onCreated={setSelected} />}

      <Card>
        <CardContent className="pt-6">
          {gdps.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Couldn&apos;t load GDPs.
            </p>
          ) : !gdps.data ? (
            <p className="py-8 text-center text-sm text-muted-foreground">Loading…</p>
          ) : gdps.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No ground delay programs.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Status</th>
                    <th className="pb-2 pr-3 font-medium">Airport</th>
                    <th className="pb-2 pr-3 font-medium">AAR</th>
                    <th className="pb-2 pr-3 font-medium">Window</th>
                    <th className="pb-2 pr-3 font-medium">Scope</th>
                    <th className="pb-2 pr-3 font-medium">Tier</th>
                    <th className="pb-2 pr-3 font-medium">Updated</th>
                    <th className="pb-2" />
                  </tr>
                </thead>
                <tbody>
                  {gdps.data.map((gdp) => (
                    <GdpRow
                      key={gdp.id}
                      gdp={gdp}
                      selected={selected === gdp.id}
                      onSelect={() => setSelected(gdp.id)}
                      canPublish={canPublish}
                      canDelete={canDelete}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      {selected && (
        <BoardView
          key={selected}
          id={selected}
          canPublish={canPublish}
          canRevise={canCreate}
        />
      )}
    </div>
  );
}

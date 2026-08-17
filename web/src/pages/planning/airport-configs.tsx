import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {Plus, Wind, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {
  type AirportConfig,
  type UpsertAirportConfig,
  useAirportConfigs,
  useCreateAirportConfig,
  useDeleteAirportConfig,
  useUpdateAirportConfig,
} from "@/lib/airport-configs";
import {hasPermission} from "@/lib/permissions";

const clampRate = (n: number) => Math.max(0, Math.min(200, Math.round(n)));
const clampDeg = (n: number) => Math.max(0, Math.min(360, Math.round(n)));
const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const BLANK: UpsertAirportConfig = {
  name: "",
  aar: 30,
  adr: 30,
  landing_runways: [],
  wind_from_deg: 0,
  wind_to_deg: 360,
  calm_default: false,
};

function ConfigForm({
  initial,
  editingId,
  onCancel,
  onSave,
  pending,
}: {
  initial: UpsertAirportConfig;
  editingId: string | null;
  onCancel: () => void;
  onSave: (body: UpsertAirportConfig) => void;
  pending: boolean;
}) {
  const [f, setF] = useState<UpsertAirportConfig>(initial);
  const set = (patch: Partial<UpsertAirportConfig>) => setF((p) => ({ ...p, ...patch }));

  return (
    <div className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <label className="col-span-2 flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Name</span>
          <Input value={f.name} onChange={(e) => set({ name: e.target.value })} placeholder="South Flow" />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">AAR</span>
          <Input
            type="number"
            value={f.aar}
            onChange={(e) => set({ aar: clampRate(Number(e.target.value) || 0) })}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">ADR</span>
          <Input
            type="number"
            value={f.adr}
            onChange={(e) => set({ adr: clampRate(Number(e.target.value) || 0) })}
          />
        </label>
      </div>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <label className="col-span-2 flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Landing runways</span>
          <Input
            value={(f.landing_runways ?? []).join(", ")}
            onChange={(e) =>
              set({ landing_runways: e.target.value.split(/[,\s]+/).map((s) => s.trim().toUpperCase()).filter(Boolean) })
            }
            placeholder="26L, 27R, 28"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Wind from °</span>
          <Input
            type="number"
            value={f.wind_from_deg}
            onChange={(e) => set({ wind_from_deg: clampDeg(Number(e.target.value) || 0) })}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Wind to °</span>
          <Input
            type="number"
            value={f.wind_to_deg}
            onChange={(e) => set({ wind_to_deg: clampDeg(Number(e.target.value) || 0) })}
          />
        </label>
      </div>
      <div className="flex items-center justify-between gap-3">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={f.calm_default}
            onChange={(e) => set({ calm_default: e.target.checked })}
          />
          Use when wind is calm / no rule matches
        </label>
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" disabled={!f.name.trim() || pending} onClick={() => onSave(f)}>
            {editingId ? "Save" : "Add config"}
          </Button>
        </div>
      </div>
    </div>
  );
}

function windLabel(c: AirportConfig): string {
  if (c.calm_default) return "calm / fallback";
  return `${String(c.wind_from_deg).padStart(3, "0")}–${String(c.wind_to_deg).padStart(3, "0")}°`;
}

function AirportConfigs({ icao }: { icao: string }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.config.update");
  const configs = useAirportConfigs(icao);
  const create = useCreateAirportConfig(icao);
  const update = useUpdateAirportConfig(icao);
  const del = useDeleteAirportConfig(icao);
  const [form, setForm] = useState<null | { id: string | null; initial: UpsertAirportConfig }>(null);

  const rows = configs.data ?? [];
  const editable = rows[0]?.editable ?? canEdit;

  const startEdit = (c: AirportConfig) =>
    setForm({
      id: c.id,
      initial: {
        name: c.name,
        aar: c.aar,
        adr: c.adr,
        landing_runways: c.landing_runways,
        wind_from_deg: c.wind_from_deg,
        wind_to_deg: c.wind_to_deg,
        calm_default: c.calm_default,
      },
    });

  const save = (body: UpsertAirportConfig) => {
    const done = () => setForm(null);
    if (form?.id) update.mutate({ id: form.id, body }, { onSuccess: done });
    else create.mutate(body, { onSuccess: done });
  };

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Wind className="size-4" />
            </span>
            <div className="flex flex-col">
              <span className="font-semibold">{icao} configurations</span>
              <span className="text-xs text-muted-foreground">
                Named runway configs with a favored-wind rule and AAR/ADR.
              </span>
            </div>
          </div>
          {editable && !form && (
            <Button size="sm" onClick={() => setForm({ id: null, initial: BLANK })}>
              <Plus className="size-3.5" />
              Add config
            </Button>
          )}
        </div>

        {form && (
          <ConfigForm
            initial={form.initial}
            editingId={form.id}
            onCancel={() => setForm(null)}
            onSave={save}
            pending={create.isPending || update.isPending}
          />
        )}

        {!configs.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No configurations for {icao} yet{editable ? " — add one above." : "."}
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Config</th>
                  <th className="pb-2 pr-3 font-medium">Wind</th>
                  <th className="pb-2 pr-3 font-medium">Runways</th>
                  <th className="pb-2 pr-3 font-medium">AAR / ADR</th>
                  {editable && <th className="pb-2" />}
                </tr>
              </thead>
              <tbody>
                {rows.map((c) => (
                  <tr key={c.id} className="border-t">
                    <td className="py-2 pr-3">
                      <span className="font-medium">{c.name}</span>
                      {c.calm_default && (
                        <Badge variant="secondary" className="ml-2">
                          calm default
                        </Badge>
                      )}
                    </td>
                    <td className="py-2 pr-3 font-mono text-xs">{windLabel(c)}</td>
                    <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
                      {c.landing_runways.join(", ") || "—"}
                    </td>
                    <td className="py-2 pr-3 tabular-nums">
                      {c.aar} / {c.adr}
                    </td>
                    {editable && (
                      <td className="py-2 text-right">
                        <div className="flex items-center justify-end gap-1">
                          <Button size="sm" variant="ghost" className="h-7 px-2" onClick={() => startEdit(c)}>
                            Edit
                          </Button>
                          <ConfirmButton
                            size="icon"
                            variant="ghost"
                            className="size-7 text-muted-foreground hover:text-destructive"
                            warn={`Delete "${c.name}"?`}
                            onConfirm={() => del.mutate(c.id)}
                          >
                            <X className="size-4" />
                          </ConfirmButton>
                        </div>
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function AirportConfigsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "events.plan.read");
  const [icao, setIcao] = useState("");
  const [entry, setEntry] = useState("");

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have event planning access yet.
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Airport configs</h1>
        <p className="text-muted-foreground">
          Default runway configurations per airport. During event planning the forecast wind picks a
          config to predict the arrival rate (which you can override on the event).
        </p>
      </div>

      <form
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          const v = normIcao(entry);
          if (v.length >= 3) setIcao(v);
        }}
      >
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Airport (ICAO)</span>
          <Input
            className="w-40 font-mono uppercase"
            value={entry}
            onChange={(e) => setEntry(normIcao(e.target.value))}
            placeholder="KATL"
          />
        </label>
        <Button type="submit" disabled={normIcao(entry).length < 3}>
          Load
        </Button>
      </form>

      {icao && <AirportConfigs key={icao} icao={icao} />}
    </div>
  );
}

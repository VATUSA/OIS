import {useMemo, useState} from "react";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  EmptyState,
  FilterBar,
  Input,
  QueryState,
  Select,
  StatusPill,
  Switch,
} from "@ois/ui";
import {ArrowLeft, Building2, Gauge, Lock, Plane, Plus, Settings2, Wind, X} from "lucide-react";

import {usePageHeader, useView} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {
  type AirportConfig,
  type UpsertAirportConfig,
  useAirportConfigs,
  useAllAirportConfigs,
  useCreateAirportConfig,
  useDeleteAirportConfig,
  useUpdateAirportConfig,
} from "@/lib/airport-configs";
import {hasPermission} from "@/lib/permissions";

const clampRate = (n: number) => Math.max(0, Math.min(200, Math.round(n)));
const clampDeg = (n: number) => Math.max(0, Math.min(360, Math.round(n)));
const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const SUBTITLE =
  "Default runway configurations per airport. During event planning the forecast wind picks a config to predict the arrival rate (which you can override on the event).";

const BLANK: UpsertAirportConfig = {
  name: "",
  aar: 30,
  adr: 30,
  landing_runways: [],
  wind_from_deg: 0,
  wind_to_deg: 360,
  calm_default: false,
};

function Field({ label, className, children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <label className={`flex flex-col gap-1 text-xs ${className ?? ""}`}>
      <span className="font-semibold text-ink-2">{label}</span>
      {children}
    </label>
  );
}

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
  // The runways field is edited as free text — parsing on every keystroke would eat the spaces and
  // commas you type between runways. Parse to the array only when saving.
  const [runwaysText, setRunwaysText] = useState(() => (initial.landing_runways ?? []).join(", "));
  const parseRunways = (s: string) =>
    s
      .split(/[,\s]+/)
      .map((r) => r.trim().toUpperCase())
      .filter(Boolean);
  const save = () => onSave({ ...f, landing_runways: parseRunways(runwaysText) });

  return (
    <Card className="flex flex-col gap-3 p-4">
      <h2 className="text-xl font-bold">{editingId ? "Edit config" : "New config"}</h2>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Name" className="col-span-2">
          <Input value={f.name} onChange={(e) => set({ name: e.target.value })} placeholder="South Flow" />
        </Field>
        <Field label="AAR">
          <Input
            type="number"
            className="font-mono"
            value={f.aar}
            onChange={(e) => set({ aar: clampRate(Number(e.target.value) || 0) })}
          />
        </Field>
        <Field label="ADR">
          <Input
            type="number"
            className="font-mono"
            value={f.adr}
            onChange={(e) => set({ adr: clampRate(Number(e.target.value) || 0) })}
          />
        </Field>
      </div>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Landing runways" className="col-span-2">
          <Input
            className="font-mono"
            value={runwaysText}
            onChange={(e) => setRunwaysText(e.target.value)}
            placeholder="26L, 27R, 28"
          />
        </Field>
        <Field label="Wind from °">
          <Input
            type="number"
            className="font-mono"
            value={f.wind_from_deg}
            onChange={(e) => set({ wind_from_deg: clampDeg(Number(e.target.value) || 0) })}
          />
        </Field>
        <Field label="Wind to °">
          <Input
            type="number"
            className="font-mono"
            value={f.wind_to_deg}
            onChange={(e) => set({ wind_to_deg: clampDeg(Number(e.target.value) || 0) })}
          />
        </Field>
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={!!f.calm_default} onCheckedChange={(v) => set({ calm_default: v })} />
          Use when wind is calm / no rule matches
        </label>
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" disabled={!f.name.trim() || pending} onClick={save}>
            {editingId ? "Save" : "Add config"}
          </Button>
        </div>
      </div>
    </Card>
  );
}

function windLabel(c: AirportConfig): string {
  if (c.calm_default) return "calm / fallback";
  return `${String(c.wind_from_deg).padStart(3, "0")}–${String(c.wind_to_deg).padStart(3, "0")}°`;
}

/** The config columns shared by every table on this page. */
const CONFIG_COLUMNS: DataColumn<AirportConfig>[] = [
  {
    accessorKey: "name",
    header: "Config",
    icon: Settings2,
    cell: (c) => (
      <span className="flex items-center gap-2 whitespace-nowrap">
        <span className="font-semibold">{c.row.original.name}</span>
        {c.row.original.calm_default && <StatusPill tone="neutral">calm default</StatusPill>}
      </span>
    ),
  },
  {
    id: "wind",
    accessorFn: (c) => (c.calm_default ? 999 : c.wind_from_deg),
    header: "Wind",
    icon: Wind,
    mono: true,
    cell: (c) => windLabel(c.row.original),
  },
  {
    id: "runways",
    accessorFn: (c) => c.landing_runways.join(", "),
    header: "Runways",
    mono: true,
    enableSorting: false,
    cell: (c) => <span className="text-ink-2">{c.getValue<string>() || "—"}</span>,
  },
  {
    accessorKey: "aar",
    header: "AAR / ADR",
    icon: Gauge,
    mono: true,
    cell: (c) => `${c.row.original.aar} / ${c.row.original.adr}`,
  },
];

function AirportConfigs({ icao, onBack }: { icao: string; onBack: () => void }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.config.update");
  const configs = useAirportConfigs(icao);
  const create = useCreateAirportConfig(icao);
  const update = useUpdateAirportConfig(icao);
  const { mutate: del } = useDeleteAirportConfig(icao);
  const [form, setForm] = useState<null | { id: string | null; initial: UpsertAirportConfig }>(null);

  const rows = configs.data ?? [];
  const editable = rows[0]?.editable ?? canEdit;

  const save = (body: UpsertAirportConfig) => {
    const done = () => setForm(null);
    if (form?.id) update.mutate({ id: form.id, body }, { onSuccess: done });
    else create.mutate(body, { onSuccess: done });
  };

  const columns = useMemo<DataColumn<AirportConfig>[]>(
    () =>
      editable
        ? [
            ...CONFIG_COLUMNS,
            {
              id: "actions",
              header: () => <span className="sr-only">Actions</span>,
              enableSorting: false,
              align: "right",
              cell: (c) => {
                const cfg = c.row.original;
                return (
                  <div className="flex items-center justify-end gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2"
                      onClick={() =>
                        setForm({
                          id: cfg.id,
                          initial: {
                            name: cfg.name,
                            aar: cfg.aar,
                            adr: cfg.adr,
                            landing_runways: cfg.landing_runways,
                            wind_from_deg: cfg.wind_from_deg,
                            wind_to_deg: cfg.wind_to_deg,
                            calm_default: cfg.calm_default,
                          },
                        })
                      }
                    >
                      Edit
                    </Button>
                    <ConfirmButton
                      size="icon"
                      variant="ghost"
                      className="size-7 text-ink-3 hover:text-danger"
                      aria-label={`Delete ${cfg.name}`}
                      warn={`Delete "${cfg.name}"?`}
                      onConfirm={() => del(cfg.id)}
                    >
                      <X className="size-4" />
                    </ConfirmButton>
                  </div>
                );
              },
            },
          ]
        : CONFIG_COLUMNS,
    [editable, del],
  );

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="size-4" />
          All airports
        </Button>
        {editable && !form && (
          <Button size="sm" className="ml-auto" onClick={() => setForm({ id: null, initial: BLANK })}>
            <Plus className="size-3.5" />
            Add config
          </Button>
        )}
      </FilterBar>

      {form && (
        <ConfigForm
          initial={form.initial}
          editingId={form.id}
          onCancel={() => setForm(null)}
          onSave={save}
          pending={create.isPending || update.isPending}
        />
      )}

      <DataTable
        label={`${icao} configurations`}
        columns={columns}
        data={rows}
        getRowId={(c) => c.id}
        rowCap={25}
        isLoading={configs.isLoading}
        isError={!configs.data && configs.isError}
        onRetry={() => configs.refetch()}
        empty={`No configurations for ${icao} yet${editable ? " — add one above." : "."}`}
      />
    </div>
  );
}

const OPEN_COLUMN = (onOpen: (icao: string) => void): DataColumn<AirportConfig> => ({
  id: "open",
  header: () => <span className="sr-only">Open</span>,
  enableSorting: false,
  align: "right",
  cell: (c) => (
    <Button size="sm" variant="ghost" className="h-7 px-2" onClick={() => onOpen(c.row.original.icao)}>
      Open
    </Button>
  ),
});

/** Every airport's configs — grouped by ICAO (one table per airport) or as one flat list. */
function AllConfigs({
  artcc,
  grouped,
  onOpen,
}: {
  artcc: string | null;
  grouped: boolean;
  onOpen: (icao: string) => void;
}) {
  const all = useAllAirportConfigs(artcc);

  const groups = useMemo(() => {
    const m = new Map<string, AirportConfig[]>();
    for (const c of all.data ?? []) {
      const list = m.get(c.icao);
      if (list) list.push(c);
      else m.set(c.icao, [c]);
    }
    return [...m.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [all.data]);

  const listColumns = useMemo<DataColumn<AirportConfig>[]>(
    () => [
      { accessorKey: "icao", header: "Airport", icon: Plane, mono: true, cellClassName: "font-semibold" },
      {
        accessorKey: "artcc",
        header: "ARTCC",
        icon: Building2,
        mono: true,
        cell: (c) => <span className="text-ink-2">{c.getValue<string>() || "—"}</span>,
      },
      ...CONFIG_COLUMNS,
      OPEN_COLUMN(onOpen),
    ],
    [onOpen],
  );
  const groupColumns = useMemo(() => [...CONFIG_COLUMNS, OPEN_COLUMN(onOpen)], [onOpen]);

  const empty = artcc ? `No configs for ${artcc}.` : "No airport configs yet.";

  if (!grouped) {
    return (
      <DataTable
        label="Airport configs"
        columns={listColumns}
        data={all.data ?? []}
        getRowId={(c) => c.id}
        initialSort={[{ id: "icao", desc: false }]}
        rowCap={25}
        isLoading={all.isLoading}
        isError={!all.data && all.isError}
        onRetry={() => all.refetch()}
        empty={empty}
      />
    );
  }

  return (
    <QueryState
      isLoading={all.isLoading}
      isError={!all.data && all.isError}
      onRetry={() => all.refetch()}
      isEmpty={groups.length === 0}
      error="Couldn't load configs."
      empty={empty}
      className="rounded-md border border-line"
    >
      <div className="flex flex-col gap-6">
        {groups.map(([icao, configs]) => (
          <section key={icao} className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-baseline gap-2">
                <h2 className="font-mono text-xl font-bold">{icao}</h2>
                <span className="text-xs text-ink-3">
                  <span className="font-mono">{configs[0].artcc || "—"}</span> ·{" "}
                  <span className="font-mono">{configs.length}</span> config{configs.length === 1 ? "" : "s"}
                </span>
              </div>
              <Button size="sm" variant="outline" onClick={() => onOpen(icao)}>
                Open
              </Button>
            </div>
            <DataTable
              label={`${icao} configurations`}
              columns={groupColumns}
              data={configs}
              getRowId={(c) => c.id}
              rowCap={25}
            />
          </section>
        ))}
      </div>
    </QueryState>
  );
}

export function AirportConfigsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "events.plan.read");
  const facilities = useFacilities();
  const view = useView();
  const [icao, setIcao] = useState("");
  const [entry, setEntry] = useState("");
  const [artcc, setArtcc] = useState("");

  usePageHeader({
    title: icao ? `${icao} configurations` : undefined,
    subtitle: icao ? "Named runway configs with a favored-wind rule and AAR/ADR." : SUBTITLE,
    views: icao || !canRead ? null : undefined,
  });

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have event planning access yet.</EmptyState>;
  }

  if (icao) {
    return <AirportConfigs key={icao} icao={icao} onBack={() => setIcao("")} />;
  }

  const artccs = (facilities.data ?? [])
    .filter((f) => f.active)
    .map((f) => f.id)
    .sort();

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <Select aria-label="ARTCC" size="sm" value={artcc} onChange={(e) => setArtcc(e.target.value)}>
          <option value="">All ARTCCs</option>
          {artccs.map((id) => (
            <option key={id} value={id}>
              {id}
            </option>
          ))}
        </Select>
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            const v = normIcao(entry);
            if (v.length >= 3) setIcao(v);
          }}
        >
          <Input
            aria-label="Open airport (ICAO)"
            className="h-8 w-40 font-mono uppercase placeholder:normal-case"
            value={entry}
            onChange={(e) => setEntry(normIcao(e.target.value))}
            placeholder="ICAO, e.g. KATL"
          />
          <Button type="submit" size="sm" disabled={normIcao(entry).length < 3}>
            Open
          </Button>
        </form>
      </FilterBar>

      <AllConfigs artcc={artcc || null} grouped={view !== "list"} onOpen={setIcao} />
    </div>
  );
}

import {useMemo, useState} from "react";
import {Button, Card, ConfirmButton, type DataColumn, DataTable, EmptyState, Input, QueryState} from "@ois/ui";
import {ArrowDownRight, ArrowUpRight, Gauge, Lock, MoveUp, Plane, Plus, X} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {
  type AircraftProfile,
  type UpsertAircraftProfile,
  formatClimb,
  formatDescent,
  parseClimb,
  parseDescent,
  profileLabel,
  useAircraftProfiles,
  useDeleteAircraftProfile,
  useUpsertAircraftProfile,
} from "@/lib/aircraft-profiles";
import {hasPermission} from "@/lib/permissions";

const normType = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);
const numOrNull = (s: string): number | null => {
  const t = s.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
};

type FormState = {
  name: string;
  climb: string;
  climb_fpm_lo: number;
  climb_fpm_hi: number;
  cruise_tas: string;
  cruise_mach: string;
  service_ceiling_ft: number;
  descent: string;
  desc_fpm: number;
};

const BLANK: FormState = {
  name: "",
  climb: "250/290",
  climb_fpm_lo: 2000,
  climb_fpm_hi: 1500,
  cruise_tas: "",
  cruise_mach: "",
  service_ceiling_ft: 41000,
  descent: "290/250",
  desc_fpm: 1800,
};

function fromProfile(p: AircraftProfile): FormState {
  return {
    name: p.name,
    climb: formatClimb(p),
    climb_fpm_lo: p.climb_fpm_lo,
    climb_fpm_hi: p.climb_fpm_hi,
    cruise_tas: p.cruise_tas == null ? "" : String(Math.round(p.cruise_tas)),
    cruise_mach: p.cruise_mach == null ? "" : String(p.cruise_mach),
    service_ceiling_ft: p.service_ceiling_ft,
    descent: formatDescent(p),
    desc_fpm: p.desc_fpm,
  };
}

/** Build the API body from the form, filling any missing schedule parts from sane fallbacks. */
function toBody(f: FormState): UpsertAircraftProfile | null {
  const c = parseClimb(f.climb);
  const d = parseDescent(f.descent);
  if (c.ias_lo == null || c.ias_hi == null || d.ias_hi == null || d.ias_lo == null) return null;
  return {
    name: f.name,
    climb_ias_lo: c.ias_lo,
    climb_ias_hi: c.ias_hi,
    climb_mach: c.mach ?? null,
    climb_fpm_lo: f.climb_fpm_lo,
    climb_fpm_hi: f.climb_fpm_hi,
    cruise_tas: numOrNull(f.cruise_tas),
    cruise_mach: numOrNull(f.cruise_mach),
    service_ceiling_ft: f.service_ceiling_ft,
    desc_mach: d.mach ?? null,
    desc_ias_hi: d.ias_hi,
    desc_ias_lo: d.ias_lo,
    desc_fpm: f.desc_fpm,
  };
}

function Field({
  label,
  children,
  wide,
}: {
  label: string;
  children: React.ReactNode;
  wide?: boolean;
}) {
  return (
    <label className={`flex flex-col gap-1 text-xs ${wide ? "col-span-2" : ""}`}>
      <span className="font-semibold text-ink-2">{label}</span>
      {children}
    </label>
  );
}

function ProfileForm({
  initial,
  isNewType,
  onCancel,
  onSave,
  pending,
}: {
  initial: FormState;
  isNewType: boolean;
  onCancel: () => void;
  onSave: (body: UpsertAircraftProfile, typeKey?: string) => void;
  pending: boolean;
}) {
  const [f, setF] = useState<FormState>(initial);
  const [typeKey, setTypeKey] = useState("");
  const set = (patch: Partial<FormState>) => setF((p) => ({ ...p, ...patch }));
  const numeric = (v: string) => Number(v) || 0;

  const save = () => {
    const body = toBody(f);
    if (!body) return;
    onSave(body, isNewType ? normType(typeKey) : undefined);
  };
  const invalid = !toBody(f) || (isNewType && normType(typeKey).length < 2);

  return (
    <Card className="flex flex-col gap-3 p-4">
      <h2 className="text-xl font-bold">{isNewType ? "New aircraft type" : "Edit profile"}</h2>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {isNewType && (
          <Field label="ICAO type">
            <Input
              className="font-mono uppercase"
              value={typeKey}
              onChange={(e) => setTypeKey(normType(e.target.value))}
              placeholder="C172"
            />
          </Field>
        )}
        <Field label="Name" wide>
          <Input value={f.name} onChange={(e) => set({ name: e.target.value })} placeholder="Cessna 172" />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Climb (IASlo/IAShi/Mach)">
          <Input
            className="font-mono"
            value={f.climb}
            onChange={(e) => set({ climb: e.target.value })}
            placeholder="250/280/.78"
          />
        </Field>
        <Field label="Climb rate < 10k (fpm)">
          <Input type="number" className="font-mono" value={f.climb_fpm_lo} onChange={(e) => set({ climb_fpm_lo: numeric(e.target.value) })} />
        </Field>
        <Field label="Climb rate > 10k (fpm)">
          <Input type="number" className="font-mono" value={f.climb_fpm_hi} onChange={(e) => set({ climb_fpm_hi: numeric(e.target.value) })} />
        </Field>
        <Field label="Service ceiling (ft)">
          <Input
            type="number"
            className="font-mono"
            value={f.service_ceiling_ft}
            onChange={(e) => set({ service_ceiling_ft: numeric(e.target.value) })}
          />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Cruise TAS (kt, optional)">
          <Input type="number" className="font-mono" value={f.cruise_tas} onChange={(e) => set({ cruise_tas: e.target.value })} placeholder="—" />
        </Field>
        <Field label="Cruise Mach (optional)">
          <Input
            className="font-mono"
            value={f.cruise_mach}
            onChange={(e) => set({ cruise_mach: e.target.value })}
            placeholder="0.78"
          />
        </Field>
        <Field label="Descent (Mach/IAShi/IASlo)">
          <Input
            className="font-mono"
            value={f.descent}
            onChange={(e) => set({ descent: e.target.value })}
            placeholder=".78/280/250"
          />
        </Field>
        <Field label="Descent rate (fpm)">
          <Input type="number" className="font-mono" value={f.desc_fpm} onChange={(e) => set({ desc_fpm: numeric(e.target.value) })} />
        </Field>
      </div>

      <div className="flex items-center justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" disabled={invalid || pending} onClick={save}>
          Save
        </Button>
      </div>
    </Card>
  );
}

const cruiseLabel = (p: AircraftProfile) =>
  p.cruise_mach != null
    ? `M${String(p.cruise_mach).replace(/^0/, "")}`
    : p.cruise_tas != null
      ? `${Math.round(p.cruise_tas)} kt`
      : "filed";

const PROFILE_COLUMNS: DataColumn<AircraftProfile>[] = [
  {
    id: "profile",
    accessorFn: (p) => profileLabel(p),
    header: "Profile",
    icon: Plane,
    cell: (c) => (
      <span className="whitespace-nowrap">
        <span className="font-semibold">{c.getValue<string>()}</span>
        {c.row.original.name && c.row.original.kind === "type" && (
          <span className="ml-2 text-xs text-ink-3">{c.row.original.name}</span>
        )}
      </span>
    ),
  },
  { id: "climb", accessorFn: (p) => formatClimb(p), header: "Climb", icon: ArrowUpRight, mono: true, enableSorting: false },
  { id: "descent", accessorFn: (p) => formatDescent(p), header: "Descent", icon: ArrowDownRight, mono: true, enableSorting: false },
  {
    id: "cruise",
    accessorFn: (p) => cruiseLabel(p),
    header: "Cruise",
    icon: Gauge,
    mono: true,
    enableSorting: false,
    cell: (c) => <span className="text-ink-2">{c.getValue<string>()}</span>,
  },
  {
    accessorKey: "service_ceiling_ft",
    header: "Ceiling",
    icon: MoveUp,
    mono: true,
    align: "right",
    cell: (c) => Math.round(c.getValue<number>()).toLocaleString(),
  },
];

const GROUPS: [string, string][] = [
  ["default", "Default"],
  ["wake", "Wake classes"],
  ["type", "Aircraft types"],
];

export function AircraftProfilesPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "flow.aircraft_profiles.read");
  const canEdit = hasPermission(me, "flow.aircraft_profiles.update");
  const profiles = useAircraftProfiles();
  const upsert = useUpsertAircraftProfile();
  const { mutate: del } = useDeleteAircraftProfile();
  // `edit` holds the row being edited (kind+key + form), or a sentinel for a new type.
  const [edit, setEdit] = useState<null | { kind: string; key: string; isNew: boolean; initial: FormState }>(null);

  const actions = useMemo(
    () =>
      canEdit && !edit ? (
        <Button size="sm" onClick={() => setEdit({ kind: "type", key: "", isNew: true, initial: BLANK })}>
          <Plus className="size-3.5" />
          Add type
        </Button>
      ) : undefined,
    [canEdit, edit],
  );
  usePageHeader({
    subtitle: "Fine-grained aircraft performance mapping (climb, cruise, and descent) for the ETA and flow model.",
    count: canRead ? (profiles.data?.length ?? null) : null,
    actions: canRead ? actions : undefined,
  });

  const columns = useMemo<DataColumn<AircraftProfile>[]>(
    () =>
      canEdit
        ? [
            ...PROFILE_COLUMNS,
            {
              id: "actions",
              header: () => <span className="sr-only">Actions</span>,
              enableSorting: false,
              align: "right",
              cell: (c) => {
                const p = c.row.original;
                return (
                  <div className="flex items-center justify-end gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2"
                      onClick={() => setEdit({ kind: p.kind, key: p.key, isNew: false, initial: fromProfile(p) })}
                    >
                      Edit
                    </Button>
                    {p.kind !== "default" && (
                      <ConfirmButton
                        size="icon"
                        variant="ghost"
                        className="size-7 text-ink-3 hover:text-danger"
                        aria-label={`Delete ${profileLabel(p)}`}
                        warn={`Delete "${profileLabel(p)}"?`}
                        onConfirm={() => del({ kind: p.kind, key: p.key })}
                      >
                        <X className="size-4" />
                      </ConfirmButton>
                    )}
                  </div>
                );
              },
            },
          ]
        : PROFILE_COLUMNS,
    [canEdit, del],
  );

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have access to aircraft performance profiles.</EmptyState>;
  }

  const rows = profiles.data ?? [];

  const save = (body: UpsertAircraftProfile, typeKey?: string) => {
    if (!edit) return;
    const key = edit.isNew ? typeKey! : edit.key;
    upsert.mutate({ kind: edit.kind, key, body }, { onSuccess: () => setEdit(null) });
  };

  return (
    <div className="flex flex-col gap-6">
      {edit && (
        <ProfileForm
          initial={edit.initial}
          isNewType={edit.isNew}
          onCancel={() => setEdit(null)}
          onSave={save}
          pending={upsert.isPending}
        />
      )}

      <QueryState
        isLoading={profiles.isLoading}
        isError={profiles.isError}
        onRetry={() => profiles.refetch()}
        className="rounded-md border border-line"
      >
        {GROUPS.map(([kind, title]) => {
          const list = rows.filter((p) => p.kind === kind);
          return list.length === 0 ? null : (
            <section key={kind} className="flex flex-col gap-2">
              <h2 className="flex items-baseline gap-2 text-xl font-bold">
                {title}
                <span className="font-mono text-sm font-normal text-ink-3">{list.length}</span>
              </h2>
              <DataTable
                label={title}
                columns={columns}
                data={list}
                getRowId={(p) => `${p.kind}/${p.key}`}
                rowCap={kind === "type" ? 25 : 10}
              />
            </section>
          );
        })}
      </QueryState>
    </div>
  );
}

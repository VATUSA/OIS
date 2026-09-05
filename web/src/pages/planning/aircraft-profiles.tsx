import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {Plane, Plus, X} from "lucide-react";

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
      <span className="text-muted-foreground">{label}</span>
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
    <div className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3">
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
          <Input type="number" value={f.climb_fpm_lo} onChange={(e) => set({ climb_fpm_lo: numeric(e.target.value) })} />
        </Field>
        <Field label="Climb rate > 10k (fpm)">
          <Input type="number" value={f.climb_fpm_hi} onChange={(e) => set({ climb_fpm_hi: numeric(e.target.value) })} />
        </Field>
        <Field label="Service ceiling (ft)">
          <Input
            type="number"
            value={f.service_ceiling_ft}
            onChange={(e) => set({ service_ceiling_ft: numeric(e.target.value) })}
          />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Cruise TAS (kt, optional)">
          <Input type="number" value={f.cruise_tas} onChange={(e) => set({ cruise_tas: e.target.value })} placeholder="—" />
        </Field>
        <Field label="Cruise Mach (optional)">
          <Input
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
          <Input type="number" value={f.desc_fpm} onChange={(e) => set({ desc_fpm: numeric(e.target.value) })} />
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
    </div>
  );
}

function ProfileRow({
  p,
  editable,
  onEdit,
  onDelete,
}: {
  p: AircraftProfile;
  editable: boolean;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <tr className="border-t">
      <td className="py-2 pr-3">
        <span className="font-medium">{profileLabel(p)}</span>
        {p.name && p.kind === "type" && (
          <span className="ml-2 text-xs text-muted-foreground">{p.name}</span>
        )}
      </td>
      <td className="py-2 pr-3 font-mono text-xs">{formatClimb(p)}</td>
      <td className="py-2 pr-3 font-mono text-xs">{formatDescent(p)}</td>
      <td className="py-2 pr-3 tabular-nums text-xs text-muted-foreground">
        {p.cruise_mach != null ? `M${String(p.cruise_mach).replace(/^0/, "")}` : p.cruise_tas != null ? `${Math.round(p.cruise_tas)} kt` : "filed"}
      </td>
      <td className="py-2 pr-3 tabular-nums">{Math.round(p.service_ceiling_ft).toLocaleString()}</td>
      {editable && (
        <td className="py-2 text-right">
          <div className="flex items-center justify-end gap-1">
            <Button size="sm" variant="ghost" className="h-7 px-2" onClick={onEdit}>
              Edit
            </Button>
            {p.kind !== "default" && (
              <ConfirmButton
                size="icon"
                variant="ghost"
                className="size-7 text-muted-foreground hover:text-destructive"
                warn={`Delete "${profileLabel(p)}"?`}
                onConfirm={onDelete}
              >
                <X className="size-4" />
              </ConfirmButton>
            )}
          </div>
        </td>
      )}
    </tr>
  );
}

export function AircraftProfilesPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "flow.aircraft_profiles.read");
  const canEdit = hasPermission(me, "flow.aircraft_profiles.update");
  const profiles = useAircraftProfiles();
  const upsert = useUpsertAircraftProfile();
  const del = useDeleteAircraftProfile();
  // `edit` holds the row being edited (kind+key + form), or a sentinel for a new type.
  const [edit, setEdit] = useState<
    null | { kind: string; key: string; isNew: boolean; initial: FormState }
  >(null);

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to aircraft performance profiles.
        </CardContent>
      </Card>
    );
  }

  const rows = profiles.data ?? [];
  const groups: [string, AircraftProfile[]][] = [
    ["Default", rows.filter((p) => p.kind === "default")],
    ["Wake classes", rows.filter((p) => p.kind === "wake")],
    ["Aircraft types", rows.filter((p) => p.kind === "type")],
  ];

  const save = (body: UpsertAircraftProfile, typeKey?: string) => {
    if (!edit) return;
    const key = edit.isNew ? typeKey! : edit.key;
    upsert.mutate({ kind: edit.kind, key, body }, { onSuccess: () => setEdit(null) });
  };

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Aircraft profiles</h1>
          <p className="text-muted-foreground">
            Fine-grained aircraft performance mapping (climb, cruise, and descent) for the ETA and
            flow model.
          </p>
        </div>
        {canEdit && !edit && (
          <Button size="sm" onClick={() => setEdit({ kind: "type", key: "", isNew: true, initial: BLANK })}>
            <Plus className="size-3.5" />
            Add type
          </Button>
        )}
      </div>

      {edit && (
        <Card>
          <CardContent className="pt-6">
            <ProfileForm
              initial={edit.initial}
              isNewType={edit.isNew}
              onCancel={() => setEdit(null)}
              onSave={save}
              pending={upsert.isPending}
            />
          </CardContent>
        </Card>
      )}

      {!profiles.data ? (
        <p className="py-2 text-sm text-muted-foreground">Loading…</p>
      ) : (
        <Card>
          <CardContent className="flex flex-col gap-6 pt-6">
            {groups.map(([title, list]) =>
              list.length === 0 ? null : (
                <div key={title} className="flex flex-col gap-2">
                  <div className="flex items-center gap-2">
                    <span className="flex size-7 items-center justify-center rounded-md bg-primary/10 text-primary">
                      <Plane className="size-4" />
                    </span>
                    <span className="font-semibold">{title}</span>
                    <Badge variant="secondary">{list.length}</Badge>
                  </div>
                  <div className="overflow-x-auto">
                    <table className="w-full text-sm">
                      <thead>
                        <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                          <th className="pb-2 pr-3 font-medium">Profile</th>
                          <th className="pb-2 pr-3 font-medium">Climb</th>
                          <th className="pb-2 pr-3 font-medium">Descent</th>
                          <th className="pb-2 pr-3 font-medium">Cruise</th>
                          <th className="pb-2 pr-3 font-medium">Ceiling</th>
                          {canEdit && <th className="pb-2" />}
                        </tr>
                      </thead>
                      <tbody>
                        {list.map((p) => (
                          <ProfileRow
                            key={`${p.kind}/${p.key}`}
                            p={p}
                            editable={canEdit}
                            onEdit={() =>
                              setEdit({ kind: p.kind, key: p.key, isNew: false, initial: fromProfile(p) })
                            }
                            onDelete={() => del.mutate({ kind: p.kind, key: p.key })}
                          />
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              ),
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}

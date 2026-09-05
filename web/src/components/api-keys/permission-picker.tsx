import {useMemo, useState} from "react";
import {ConfirmButton, cn} from "@ois/ui";
import {Wand2} from "lucide-react";

import type {ApiKeyPermission, ApiKeyPermissionInput, GrantablePermission} from "@/lib/api-keys";
import {type AccessPreset, ACCESS_PRESETS, BASE_PERMISSIONS, presetPermissions} from "@/lib/presets";
import {
  PermissionScopeTree,
  type ScopeItem,
  type ScopeSel,
  type ScopeSelection,
  defaultScope,
  selectionIsValid as scopeSelectionIsValid,
} from "@/components/access/scope-tree";

// Re-exported for the API-keys page (the selection is a name → scope map).
export type {ScopeSel};
export type PermSelection = ScopeSelection;

/** Flatten the picker's selection into the request's `(permission, artcc)` grants. */
export function buildPermissionInputs(selection: PermSelection): ApiKeyPermissionInput[] {
  const out: ApiKeyPermissionInput[] = [];
  for (const [permission, s] of selection) {
    if (s.national) out.push({ permission, artcc_id: null });
    else for (const artcc_id of s.artccs) out.push({ permission, artcc_id });
  }
  return out;
}

/** Seed a selection from a key's existing grants (for the edit flow). */
export function selectionFromPermissions(perms: ApiKeyPermission[]): PermSelection {
  const m: PermSelection = new Map();
  for (const p of perms) {
    const cur = m.get(p.permission) ?? { national: false, artccs: [] };
    if (p.artcc_id == null) cur.national = true;
    else if (!cur.artccs.includes(p.artcc_id)) cur.artccs.push(p.artcc_id);
    m.set(p.permission, cur);
  }
  return m;
}

/** Whether the selection is valid to submit (every scoped entry has at least one ARTCC). */
export function selectionIsValid(selection: PermSelection): boolean {
  return scopeSelectionIsValid(selection);
}

/**
 * Permission picker for API keys: a presets bar over the shared grouped scope tree. Each checked
 * permission gets a scope control (National or specific ARTCCs) bounded by what the caller can
 * delegate (`grantable`).
 */
export function PermissionPicker({
  grantable,
  facilities,
  selection,
  onChange,
}: {
  grantable: GrantablePermission[];
  facilities: { id: string; name: string }[];
  selection: PermSelection;
  onChange: (next: PermSelection) => void;
}) {
  const [presetFacility, setPresetFacility] = useState(""); // for facility presets

  // --- Presets: bundle the caller's grantable permissions. Keys hold no roles, so preset.roles is
  // ignored; a preset can never exceed what the caller can delegate (it's drawn from `grantable`).
  const grantableByName = useMemo(
    () => new Map(grantable.map((g) => [g.permission, g] as const)),
    [grantable],
  );
  const grantableNames = useMemo(() => grantable.map((g) => g.permission), [grantable]);
  // The sign-in baseline (BASE_PERMISSIONS) — always national — plus the preset's domain perms at its
  // scope. Keys hold no roles, so this is the only way a preset key gets the defaults a user has.
  const baseNames = useMemo(
    () => BASE_PERMISSIONS.filter((p) => grantableByName.has(p)),
    [grantableByName],
  );
  const scopeOf = (g: GrantablePermission): ScopeSel =>
    defaultScope({ national: g.national, artccs: g.artccs });
  const isPresetApplied = (preset: AccessPreset) => {
    const perms = [...presetPermissions(preset, grantableNames), ...baseNames];
    return perms.length > 0 && perms.every((p) => selection.has(p));
  };
  const togglePreset = (preset: AccessPreset) => {
    const domain = presetPermissions(preset, grantableNames);
    const next = new Map(selection);
    if (isPresetApplied(preset)) {
      for (const p of [...domain, ...baseNames]) next.delete(p);
    } else {
      for (const p of domain) {
        const g = grantableByName.get(p);
        if (!g) continue;
        if (preset.scope === "facility") {
          // Scope to the chosen facility, only where the caller can actually delegate it.
          if (presetFacility && (g.national || g.artccs.includes(presetFacility))) {
            next.set(p, { national: false, artccs: [presetFacility] });
          }
        } else {
          next.set(p, scopeOf(g)); // national where held nationally, else all their ARTCCs
        }
      }
      // Baseline is always national (applied after domain so it wins for any overlap — national ⊇ facility).
      for (const p of baseNames) next.set(p, scopeOf(grantableByName.get(p)!));
    }
    onChange(next);
  };

  const items: ScopeItem[] = useMemo(
    () =>
      grantable.map((g) => ({
        name: g.permission,
        bounds: { national: g.national, artccs: g.artccs },
      })),
    [grantable],
  );

  if (grantable.length === 0) {
    return (
      <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
        You don&apos;t hold any permissions that can be delegated to a key.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {/* Presets — bundle grantable permissions (no roles on keys). */}
      <div className="flex flex-col gap-1.5 rounded-md border bg-muted/20 p-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
            <Wand2 className="size-3.5 text-primary" /> Presets
          </span>
          {ACCESS_PRESETS.map((preset) => {
            const needsFacility = preset.scope === "facility" && !presetFacility;
            const applied = isPresetApplied(preset);
            return (
              <button
                key={preset.id}
                type="button"
                disabled={needsFacility}
                onClick={() => togglePreset(preset)}
                title={needsFacility ? "Pick a facility first" : preset.description}
                className={cn(
                  "rounded-md border px-2.5 py-1 text-xs font-medium transition-colors",
                  applied
                    ? "border-primary/60 bg-primary/15 text-primary"
                    : "bg-background hover:bg-accent",
                  needsFacility && "cursor-not-allowed opacity-50",
                )}
              >
                {preset.label}
                {preset.scope === "facility" && presetFacility ? ` · ${presetFacility}` : ""}
              </button>
            );
          })}
          <select
            value={presetFacility}
            onChange={(e) => setPresetFacility(e.target.value)}
            title="Facility for the EC preset"
            className="h-7 rounded-md border border-input bg-background px-2 text-xs"
          >
            <option value="">Facility…</option>
            {facilities.map((f) => (
              <option key={f.id} value={f.id}>
                {f.id}
              </option>
            ))}
          </select>
          <span className="mx-0.5 h-5 w-px bg-border" />
          <ConfirmButton
            size="sm"
            variant="ghost"
            warn="Clear every permission on this key?"
            onConfirm={() => onChange(new Map())}
          >
            Remove all
          </ConfirmButton>
        </div>
      </div>

      <PermissionScopeTree
        items={items}
        facilities={facilities}
        selection={selection}
        onChange={onChange}
      />
    </div>
  );
}

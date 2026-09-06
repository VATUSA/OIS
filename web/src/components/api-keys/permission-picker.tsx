import {useMemo, useState} from "react";

import type {ApiKeyPermission, ApiKeyPermissionInput, GrantablePermission} from "@/lib/api-keys";
import {type AccessPreset, BASE_PERMISSIONS, presetPermissions} from "@/lib/presets";
import {PresetBar} from "@/components/access/preset-bar";
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
      <PresetBar
        isApplied={isPresetApplied}
        onToggle={togglePreset}
        facility={presetFacility}
        facilities={facilities}
        onFacility={setPresetFacility}
        onRemoveAll={() => onChange(new Map())}
        removeAllWarn="Clear every permission on this key?"
      />
      <PermissionScopeTree
        items={items}
        facilities={facilities}
        selection={selection}
        onChange={onChange}
      />
    </div>
  );
}

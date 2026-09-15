import {useMemo, useState} from "react";

import type {ApiKeyPermission, ApiKeyPermissionInput, GrantablePermission} from "@/lib/api-keys";
import {ACCESS_PRESETS, type AccessPreset, BASE_PERMISSIONS, presetPermissions} from "@/lib/presets";
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
 * What a preset grants this creator beyond the sign-in baseline — the same set `togglePreset`
 * writes. The baseline is excluded because every preset adds it: counting it made a preset with
 * nothing of its own (e.g. ACE Team for a creator holding no `ace.*` beyond the baseline) read as
 * applied as soon as any other preset was clicked (#264). A facility preset grants nothing until a
 * facility is chosen, and then only what the creator can delegate at it.
 */
export function presetOwnPermissions(
  preset: AccessPreset,
  grantable: GrantablePermission[],
  baseNames: readonly string[],
  facility: string,
): string[] {
  if (preset.scope === "facility" && !facility) return [];
  const byName = new Map(grantable.map((g) => [g.permission, g] as const));
  const base = new Set(baseNames);
  return presetPermissions(
    preset,
    grantable.map((g) => g.permission),
  ).filter((p) => {
    if (base.has(p)) return false;
    if (preset.scope !== "facility") return true;
    const g = byName.get(p)!;
    return g.national || g.artccs.includes(facility);
  });
}

/** Whether `s` covers the full scope a creator holds a permission at — what `togglePreset` writes
 * for a national preset (`defaultScope`): national if held nationally, else every held ARTCC. */
function coversHeldScope(s: ScopeSel, g: GrantablePermission): boolean {
  return s.national || (!g.national && g.artccs.every((a) => s.artccs.includes(a)));
}

/** Whether a preset is fully applied: it grants something of its own, all of it is selected at the
 * scope the preset writes (a facility preset's at the chosen facility, not nationally; a national
 * preset's at the creator's full held scope — so a facility preset's ARTCC-scoped grants never make
 * a national preset read as applied, #264), and so is the baseline. */
export function presetApplied(
  preset: AccessPreset,
  grantable: GrantablePermission[],
  baseNames: readonly string[],
  facility: string,
  selection: PermSelection,
): boolean {
  const own = presetOwnPermissions(preset, grantable, baseNames, facility);
  if (own.length === 0) return false;
  const byName = new Map(grantable.map((g) => [g.permission, g] as const));
  const ownSelected = own.every((p) => {
    const s = selection.get(p);
    if (!s) return false;
    return preset.scope === "facility"
      ? !s.national && s.artccs.includes(facility)
      : coversHeldScope(s, byName.get(p)!);
  });
  return ownSelected && baseNames.every((p) => selection.has(p));
}

/** Whether a preset's chip is enabled: it grants something of its own. A facility preset with no
 * facility chosen stays enabled here — PresetBar already gates it with "Pick a facility first". */
export function presetCanApply(
  preset: AccessPreset,
  grantable: GrantablePermission[],
  baseNames: readonly string[],
  facility: string,
): boolean {
  return (
    (preset.scope === "facility" && !facility) ||
    presetOwnPermissions(preset, grantable, baseNames, facility).length > 0
  );
}

/** The selection after clicking a preset. Applied: subtracts what it contributes (a facility
 * preset's ARTCC, a national preset's perms), dropping the baseline only once nothing still needs
 * it. Otherwise: merges its perms in at the preset's scope without narrowing existing grants (#275). */
export function togglePresetSelection(
  preset: AccessPreset,
  grantable: GrantablePermission[],
  baseNames: readonly string[],
  facility: string,
  selection: PermSelection,
): PermSelection {
  const byName = new Map(grantable.map((g) => [g.permission, g] as const));
  const own = presetOwnPermissions(preset, grantable, baseNames, facility);
  const next = new Map(selection);
  if (presetApplied(preset, grantable, baseNames, facility, selection)) {
    for (const p of own) {
      const artccs = next.get(p)!.artccs.filter((a) => a !== facility);
      if (preset.scope === "facility" && artccs.length > 0) next.set(p, { national: false, artccs });
      else next.delete(p);
    }
    // Keep the baseline while this preset still grants at another ARTCC or another preset is applied.
    const stillNeeded =
      own.some((p) => next.has(p)) ||
      ACCESS_PRESETS.some((o) => o.id !== preset.id && presetApplied(o, grantable, baseNames, facility, next));
    if (!stillNeeded) for (const p of baseNames) next.delete(p);
    return next;
  }
  for (const p of own) {
    if (preset.scope === "facility") {
      // Add the chosen facility to what's selected; a national grant already covers it.
      const s = next.get(p);
      if (s?.national) continue;
      const artccs = s?.artccs ?? [];
      next.set(p, { national: false, artccs: artccs.includes(facility) ? artccs : [...artccs, facility] });
    } else {
      next.set(p, defaultScope(byName.get(p)!)); // national where held nationally, else all their ARTCCs
    }
  }
  // Baseline is always national (applied after domain so it wins for any overlap — national ⊇ facility).
  for (const p of baseNames) next.set(p, defaultScope(byName.get(p)!));
  return next;
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
  // The sign-in baseline (BASE_PERMISSIONS) — always national — plus the preset's domain perms at its
  // scope. Keys hold no roles, so this is the only way a preset key gets the defaults a user has.
  const baseNames = useMemo(
    () => BASE_PERMISSIONS.filter((p) => grantableByName.has(p)),
    [grantableByName],
  );
  const isPresetApplied = (preset: AccessPreset) =>
    presetApplied(preset, grantable, baseNames, presetFacility, selection);
  const canApplyPreset = (preset: AccessPreset) =>
    presetCanApply(preset, grantable, baseNames, presetFacility);
  const togglePreset = (preset: AccessPreset) =>
    onChange(togglePresetSelection(preset, grantable, baseNames, presetFacility, selection));

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
        canApply={canApplyPreset}
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

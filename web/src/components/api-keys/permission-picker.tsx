import {useMemo} from "react";
import {ConfirmButton, Select} from "@ois/ui";

import type {ApiKeyPermission, ApiKeyPermissionInput, GrantablePermission} from "@/lib/api-keys";
import {type HeldGroup, useHeldGroups} from "@/lib/groups";
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
 * The selection after starting from `group` (#550).
 *
 * A **one-way merge**: each of the group's permissions the caller can delegate is added at its
 * default scope, and **anything already selected is left exactly as it is**. That is the whole fix for
 * the two bugs presets had:
 *
 * - **#275** — stacking presets silently narrowed scopes, because applying one *rewrote* an existing
 *   grant at its own scope. Here an existing entry is never touched, so it can only stay as broad as
 *   it was.
 * - **#264** — "is this preset applied?" was reverse-engineered from the selection, and any preset
 *   selecting all permissions read as applied. There is no applied state here at all: starting from a
 *   group is an action, not a toggle, so there is nothing to infer.
 *
 * A permission the group grants but the caller cannot delegate is skipped: `grantable` is already
 * the cap, and the server re-checks against the owner's live access when the key is created.
 */
export function mergeGroupIntoSelection(
  group: HeldGroup,
  grantable: GrantablePermission[],
  selection: PermSelection,
): PermSelection {
  const byName = new Map(grantable.map((g) => [g.permission, g] as const));
  const next = new Map(selection);
  for (const permission of group.permissions) {
    const g = byName.get(permission);
    if (!g || next.has(permission)) continue;
    next.set(permission, defaultScope(g));
  }
  return next;
}

/**
 * Permission picker for API keys: a "start from a group" control over the shared grouped scope tree. Each checked
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
  const held = useHeldGroups();
  // Only groups that would add something: a group whose permissions are all undelegable, or already
  // selected, is noise in the list.
  const startable = useMemo(() => {
    const delegable = new Set(grantable.map((g) => g.permission));
    return (held.data ?? []).filter((group) =>
      group.permissions.some((p) => delegable.has(p) && !selection.has(p)),
    );
  }, [held.data, grantable, selection]);

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
      <p className="rounded-md border border-line bg-panel-2 p-4 text-center text-sm text-ink-2">
        You don&apos;t hold any permissions that can be delegated to a key.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-panel p-2.5">
        {/* An action, not a toggle: choosing a group merges it in and the control resets. There is
            deliberately no "applied" state to show — inferring one is what #264 was. */}
        <Select
          size="sm"
          value=""
          disabled={startable.length === 0}
          onChange={(e) => {
            const group = startable.find((g) => g.name === e.target.value);
            if (group) onChange(mergeGroupIntoSelection(group, grantable, selection));
          }}
          aria-label="Start from one of your groups"
          title="Add every permission one of your groups grants"
        >
          <option value="">
            {held.isLoading
              ? "Loading your groups…"
              : startable.length === 0
                ? "No group adds anything"
                : "Start from a group…"}
          </option>
          {startable.map((g) => (
            <option key={g.name} value={g.name}>
              {g.name}
            </option>
          ))}
        </Select>
        <ConfirmButton
          size="sm"
          variant="ghost"
          className="ml-auto"
          warn="Clear every permission on this key?"
          onConfirm={() => onChange(new Map())}
        >
          Remove all
        </ConfirmButton>
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

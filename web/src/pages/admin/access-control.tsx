import {useEffect, useMemo, useState} from "react";
import {
  Button,
  Card,
  type DataColumn,
  DataTable,
  FilterBar,
  Input,
  QueryState,
  StatusPill,
} from "@ois/ui";
import {Award, Hash, Save, Search, ShieldCheck, User} from "lucide-react";

import {
  type AdminUserRow,
  buildTree,
  flattenTree,
  type PermTree,
  type UpdateBody,
  useAllUsers,
  useCatalog,
  useSaveUserAccess,
  useUserAccess,
} from "@/lib/access";
import {type AccessPreset, BASE_PERMISSIONS, presetPermissions} from "@/lib/presets";
import {PresetBar} from "@/components/access/preset-bar";
import {usePageHeader} from "@/components/shell/page-meta";
import {
  PermissionScopeTree,
  ScopeChips,
  type ScopeBounds,
  type ScopeSel,
  type ScopeSelection,
  defaultScope,
  selectionIsValid,
} from "@/components/access/scope-tree";

const USERS_PAGE_SIZE = 25;
const SUBTITLE = "Grant roles and fine-grained, per-ARTCC permissions. Every change is audited.";

/** National scope key is the empty string; anything else is an ARTCC id. */
type ScopeKey = string;

function saveErrorMessage(error: unknown): string {
  const status = (error as { status?: number } | null)?.status;
  if (status === 401 || status === 403) {
    return "You can only grant roles and permissions you hold yourself.";
  }
  if (status === 404) return "User not found.";
  return "Save failed.";
}

/** Does a selection grant `name` at the given scope (""=national)? */
function hasScope(sel: ScopeSel | undefined, key: ScopeKey): boolean {
  if (!sel) return false;
  return key === "" ? sel.national : sel.artccs.includes(key);
}

/** Return `sel` with the scope `key` added/removed; `undefined` when nothing is left selected. */
function withScope(sel: ScopeSel | undefined, key: ScopeKey, on: boolean): ScopeSel | undefined {
  let national = sel?.national ?? false;
  let artccs = sel ? [...sel.artccs] : [];
  if (key === "") {
    national = on;
  } else if (on) {
    if (!artccs.includes(key)) artccs.push(key);
  } else {
    artccs = artccs.filter((a) => a !== key);
  }
  if (!national && artccs.length === 0) return undefined;
  return { national, artccs };
}

/** All scope keys touched by a selection (""=national plus any ARTCC). */
function scopeKeysOf(selection: ScopeSelection): Set<ScopeKey> {
  const keys = new Set<ScopeKey>();
  for (const s of selection.values()) {
    if (s.national) keys.add("");
    for (const a of s.artccs) keys.add(a);
  }
  return keys;
}

/** Names in a selection granted at scope `key`. */
function namesAtScope(selection: ScopeSelection, key: ScopeKey): string[] {
  const out: string[] = [];
  for (const [name, s] of selection) if (hasScope(s, key)) out.push(name);
  return out;
}

export function AdminAccessControl() {
  const [cid, setCid] = useState<number | undefined>(undefined);
  const [userQuery, setUserQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [usersPage, setUsersPage] = useState(1);
  const [selected, setSelected] = useState<{ cid: number; name: string } | null>(null);

  const catalog = useCatalog();
  const access = useUserAccess(cid);
  const save = useSaveUserAccess();

  // The working selection: each permission / role mapped to the scope(s) it's granted at.
  const [permSel, setPermSel] = useState<ScopeSelection>(new Map());
  const [roleSel, setRoleSel] = useState<ScopeSelection>(new Map());
  const [reason, setReason] = useState("");
  const [presetFacility, setPresetFacility] = useState(""); // for facility presets

  // Scope keys present when this user's access loaded — always re-sent on save so a scope emptied
  // in the UI is actually cleared server-side (untouched scopes are otherwise preserved).
  const [originalScopeKeys, setOriginalScopeKeys] = useState<ScopeKey[]>([]);

  useEffect(() => {
    if (!access.data) return;
    const perms: ScopeSelection = new Map();
    const roles: ScopeSelection = new Map();
    const original = new Set<ScopeKey>([""]);
    for (const s of access.data.scopes) {
      const key = s.artcc_id ?? "";
      original.add(key);
      for (const r of s.role_names) roles.set(r, withScope(roles.get(r), key, true)!);
      for (const p of flattenTree(s.permissions as PermTree)) {
        perms.set(p, withScope(perms.get(p), key, true)!);
      }
    }
    setPermSel(perms);
    setRoleSel(roles);
    setOriginalScopeKeys([...original]);
    setReason("");
    setPresetFacility("");
    save.reset();
  }, [access.data]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(userQuery.trim()), 200);
    return () => clearTimeout(timer);
  }, [userQuery]);
  useEffect(() => setUsersPage(1), [debouncedQuery]);
  const users = useAllUsers(usersPage, USERS_PAGE_SIZE, debouncedQuery);
  usePageHeader({ subtitle: SUBTITLE, count: users.data?.total ?? null });

  // Server admins hold every permission implicitly — the permission tree is read-only, but roles
  // are still editable.
  const permsReadOnly = !!access.data?.server_admin;

  const facilities = useMemo(() => catalog.data?.facilities ?? [], [catalog.data]);
  // An admin may grant any catalog item nationally or at any facility; the server enforces the
  // self-scope guard (you can only grant what you hold).
  const anyScope: ScopeBounds = useMemo(
    () => ({ national: true, artccs: facilities.map((f) => f.id) }),
    [facilities],
  );

  const allPerms = useMemo(
    () => flattenTree((catalog.data?.permissions ?? {}) as PermTree).sort(),
    [catalog.data],
  );
  const permItems = useMemo(
    () => allPerms.map((name) => ({ name, bounds: anyScope })),
    [allPerms, anyScope],
  );
  const assignableRoles = useMemo(() => catalog.data?.roles ?? [], [catalog.data]);

  // --- Presets: one-click bundles toggled on/off. National presets apply at national scope;
  // facility presets apply at the facility chosen here. Every preset also grants the sign-in
  // baseline (BASE_PERMISSIONS) at national scope.
  function presetTarget(preset: AccessPreset): ScopeKey | null {
    if (preset.scope === "national") return "";
    return presetFacility || null; // facility preset needs a facility chosen
  }
  /** Preset contributions: {perms, roles} at the target scope + baseline perms at national. */
  function presetParts(
    preset: AccessPreset,
    target: ScopeKey,
  ): { key: ScopeKey; perms: string[]; roles: string[] }[] {
    const domain = presetPermissions(preset, allPerms);
    const base = [...BASE_PERMISSIONS];
    if (target === "") {
      return [{ key: "", perms: [...new Set([...domain, ...base])], roles: preset.roles }];
    }
    return [
      { key: target, perms: domain, roles: preset.roles },
      { key: "", perms: base, roles: [] },
    ];
  }
  function isPresetApplied(preset: AccessPreset): boolean {
    const target = presetTarget(preset);
    if (target == null) return false;
    return presetParts(preset, target).every(({ key, perms, roles }) => {
      return (
        perms.every((p) => hasScope(permSel.get(p), key)) &&
        roles.every((r) => hasScope(roleSel.get(r), key))
      );
    });
  }
  function togglePreset(preset: AccessPreset) {
    const target = presetTarget(preset);
    if (target == null) return;
    const on = !isPresetApplied(preset);
    const parts = presetParts(preset, target);
    setPermSel((prev) => {
      const next = new Map(prev);
      for (const { key, perms } of parts) {
        for (const p of perms) {
          const s = withScope(next.get(p), key, on);
          if (s) next.set(p, s);
          else next.delete(p);
        }
      }
      return next;
    });
    setRoleSel((prev) => {
      const next = new Map(prev);
      for (const { key, roles } of parts) {
        for (const r of roles) {
          const s = withScope(next.get(r), key, on);
          if (s) next.set(r, s);
          else next.delete(r);
        }
      }
      return next;
    });
  }

  function toggleRole(role: string, on: boolean) {
    setRoleSel((prev) => {
      const next = new Map(prev);
      if (on) next.set(role, defaultScope(anyScope));
      else next.delete(role);
      return next;
    });
  }
  function setRoleScope(role: string, s: ScopeSel) {
    setRoleSel((prev) => new Map(prev).set(role, s));
  }
  function removeAll() {
    setPermSel(new Map());
    setRoleSel(new Map());
  }

  function pickUser(user: AdminUserRow) {
    setSelected({ cid: user.cid, name: user.display_name });
    setCid(user.cid);
  }

  const valid =
    !!reason.trim() && selectionIsValid(permSel) && selectionIsValid(roleSel);

  function onSave() {
    if (cid == null || !valid) return;
    const assignable = new Set(assignableRoles);
    // Re-send every scope that existed on load or is selected now, so cleared scopes are applied.
    const keys = new Set<ScopeKey>([
      "",
      ...originalScopeKeys,
      ...scopeKeysOf(permSel),
      ...scopeKeysOf(roleSel),
    ]);
    const scopes = [...keys].map((key) => ({
      artcc_id: key === "" ? null : key,
      permissions: (permsReadOnly
        ? {}
        : buildTree(namesAtScope(permSel, key))) as unknown as Record<string, never>,
      role_names: namesAtScope(roleSel, key).filter((r) => assignable.has(r)),
    }));
    save.mutate({ cid, body: { reason: reason.trim(), scopes } satisfies UpdateBody });
  }

  const userColumns = useMemo<DataColumn<AdminUserRow>[]>(
    () => [
      {
        accessorKey: "display_name",
        header: "Name",
        icon: User,
        cell: (c) => <span className="whitespace-nowrap font-semibold">{c.getValue<string>()}</span>,
      },
      { accessorKey: "cid", header: "CID", icon: Hash, mono: true },
      {
        accessorKey: "rating",
        header: "Rating",
        icon: Award,
        cell: (c) => <span className="text-ink-2">{c.getValue<string | null>() ?? "—"}</span>,
      },
      {
        accessorKey: "roles",
        header: "Roles",
        icon: ShieldCheck,
        enableSorting: false,
        cell: (c) => {
          const roles = c.getValue<string[]>();
          return roles.length === 0 ? (
            <span className="text-xs text-ink-3">—</span>
          ) : (
            <div className="flex flex-wrap gap-1">
              {roles.map((r) => (
                <StatusPill key={r} tone="neutral">
                  {r}
                </StatusPill>
              ))}
            </div>
          );
        },
      },
    ],
    [],
  );

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <div className="relative w-full max-w-md">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-ink-3" />
          <Input
            aria-label="Search users"
            value={userQuery}
            onChange={(e) => setUserQuery(e.target.value)}
            placeholder="Search by name or CID"
            className="rounded-full pl-9"
          />
        </div>
      </FilterBar>

      <DataTable
        label="Users"
        columns={userColumns}
        data={users.data?.items ?? []}
        getRowId={(u) => String(u.cid)}
        rowCap={USERS_PAGE_SIZE}
        selection={{ mode: "single", selected: cid != null ? String(cid) : null, onChange: () => {} }}
        onRowClick={pickUser}
        serverPagination={
          users.data
            ? {
                page: users.data.page,
                pageSize: users.data.page_size,
                total: users.data.total,
                onPageChange: setUsersPage,
              }
            : undefined
        }
        isLoading={users.isLoading}
        isError={users.isError}
        onRetry={() => users.refetch()}
        empty={debouncedQuery ? "No users match." : "No users yet."}
      />

      {cid != null && (access.isError || access.isLoading) && (
        <Card>
          <QueryState
            isLoading={access.isLoading}
            isError={access.isError}
            error={
              (access.error as { status?: number } | null)?.status === 404
                ? "No such user — they must have signed in to OIS at least once."
                : "Couldn't load this user's access."
            }
          />
        </Card>
      )}

      {access.data && (
        <Card className="mt-2 flex flex-col gap-6 p-5">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-xl font-bold">{selected?.name ?? `CID ${access.data.cid}`}</h2>
            <span className="font-mono text-sm text-ink-3">{access.data.cid}</span>
            {access.data.server_admin && <StatusPill tone="good">Server admin</StatusPill>}
          </div>

          {/* Presets — one-click bundles, toggled on/off. Each grants its role + perms at the
              chosen scope, plus the sign-in baseline nationally. */}
          <section className="flex flex-col gap-2">
            <PresetBar
              isApplied={isPresetApplied}
              onToggle={togglePreset}
              facility={presetFacility}
              facilities={facilities}
              onFacility={setPresetFacility}
              onRemoveAll={removeAll}
              removeAllWarn="Clear every role and permission for this user (all scopes)?"
              size="lg"
            />
            <p className="text-xs text-ink-3">
              Each permission and role below is granted at National scope or specific ARTCCs — pick
              the scope under each one. Nothing is saved until you enter a reason and hit Save.
            </p>
          </section>

          {permsReadOnly && (
            <div className="rounded-md border border-line bg-brand-soft px-3 py-2 text-sm text-ink-2">
              Server admins hold every permission implicitly (managed via{" "}
              <code className="font-mono">OIS_SERVER_ADMIN_CID</code>), so the permission tree is
              read-only. Roles can still be changed.
            </div>
          )}

          {/* Roles — each with its own scope chips. */}
          <section className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold">Roles</h3>
            <div className="grid gap-1.5 sm:grid-cols-2">
              {assignableRoles.map((role) => {
                const sel = roleSel.get(role);
                return (
                  <div key={role} className="rounded-xs px-2 py-1">
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        className="size-4 accent-brand"
                        checked={!!sel}
                        onChange={(e) => toggleRole(role, e.target.checked)}
                      />
                      {role}
                    </label>
                    {sel && (
                      <ScopeChips
                        bounds={anyScope}
                        sel={sel}
                        facilities={facilities}
                        onChange={(s) => setRoleScope(role, s)}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          </section>

          {/* Direct permissions — the shared scope tree (same as the API-key editor). */}
          <section className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold">Direct permissions</h3>
            <PermissionScopeTree
              items={permItems}
              facilities={facilities}
              selection={permSel}
              disabled={permsReadOnly}
              onChange={setPermSel}
            />
          </section>

          {/* Reason + save */}
          <section className="flex flex-col gap-2 border-t border-line pt-4">
            <label htmlFor="access-reason" className="text-sm font-semibold">
              Reason
            </label>
            <Input
              id="access-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              placeholder="Recorded as a dossier entry on this controller's log"
            />
            <div className="flex items-center gap-3">
              <Button onClick={onSave} disabled={!valid || save.isPending}>
                <Save />
                {save.isPending ? "Saving…" : "Save"}
              </Button>
              {save.isSuccess && <span className="text-sm text-success">Saved.</span>}
              {save.isError && (
                <span className="text-sm text-danger">{saveErrorMessage(save.error)}</span>
              )}
            </div>
          </section>
        </Card>
      )}
    </div>
  );
}

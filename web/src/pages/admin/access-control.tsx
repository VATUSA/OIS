import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, cn, Input} from "@ois/ui";
import {Save, Search} from "lucide-react";

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
import {Pagination} from "@/components/pagination";
import {PresetBar} from "@/components/access/preset-bar";
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

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Access Control</h1>
        <p className="text-muted-foreground">
          Grant roles and fine-grained, per-ARTCC permissions. Every change is audited.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <div className="flex flex-col gap-1.5">
            <label className="text-sm font-medium">Users</label>
            <div className="relative w-full max-w-md">
              <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
              <Input
                value={userQuery}
                onChange={(e) => setUserQuery(e.target.value)}
                placeholder="Search by name or CID"
                className="pl-8"
              />
            </div>
          </div>

          {users.data ? (
            users.data.items.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                {debouncedQuery ? "No users match." : "No users yet."}
              </p>
            ) : (
              <>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <th className="py-2 pr-4 font-medium">Name</th>
                        <th className="py-2 pr-4 font-medium">CID</th>
                        <th className="py-2 pr-4 font-medium">Rating</th>
                        <th className="py-2 pr-4 font-medium">Roles</th>
                      </tr>
                    </thead>
                    <tbody>
                      {users.data.items.map((u) => (
                        <tr
                          key={u.cid}
                          onClick={() => pickUser(u)}
                          className={cn(
                            "cursor-pointer border-b last:border-0 hover:bg-accent",
                            selected?.cid === u.cid && "bg-accent",
                          )}
                        >
                          <td className="py-2 pr-4 font-medium">{u.display_name}</td>
                          <td className="whitespace-nowrap py-2 pr-4 font-mono text-xs text-muted-foreground">
                            {u.cid}
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">{u.rating ?? "—"}</td>
                          <td className="py-2 pr-4">
                            <div className="flex flex-wrap gap-1">
                              {u.roles.length === 0 ? (
                                <span className="text-xs text-muted-foreground">—</span>
                              ) : (
                                u.roles.map((r) => (
                                  <Badge key={r} variant="secondary">
                                    {r}
                                  </Badge>
                                ))
                              )}
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <Pagination
                  page={users.data.page}
                  pageSize={users.data.page_size}
                  total={users.data.total}
                  onPageChange={setUsersPage}
                />
              </>
            )
          ) : users.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">Couldn&apos;t load users.</p>
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
          )}
        </CardContent>
      </Card>

      {cid != null && access.isError && (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            {(access.error as { status?: number } | null)?.status === 404
              ? "No such user — they must have signed in to OIS at least once."
              : "Couldn't load this user's access."}
          </CardContent>
        </Card>
      )}

      {cid != null && access.isLoading && (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            Loading…
          </CardContent>
        </Card>
      )}

      {access.data && (
        <Card>
          <CardContent className="flex flex-col gap-6 pt-6">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-lg font-semibold">
                {selected?.name ?? `CID ${access.data.cid}`}
              </span>
              <span className="text-sm text-muted-foreground">CID {access.data.cid}</span>
              {access.data.server_admin && <Badge variant="success">Server admin</Badge>}
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
              <p className="text-xs text-muted-foreground">
                Each permission and role below is granted at National scope or specific ARTCCs — pick
                the scope under each one. Nothing is saved until you enter a reason and hit Save.
              </p>
            </section>

            {permsReadOnly && (
              <div className="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-sm text-muted-foreground">
                Server admins hold every permission implicitly (managed via{" "}
                <code>OIS_SERVER_ADMIN_CID</code>), so the permission tree is read-only. Roles can
                still be changed.
              </div>
            )}

            {/* Roles — each with its own scope chips. */}
            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-semibold">Roles</h3>
              <div className="grid gap-1.5 sm:grid-cols-2">
                {assignableRoles.map((role) => {
                  const sel = roleSel.get(role);
                  return (
                    <div key={role} className="rounded-md px-2 py-1">
                      <label className="flex items-center gap-2 text-sm">
                        <input
                          type="checkbox"
                          className="size-4 accent-primary"
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
            <section className="flex flex-col gap-2 border-t pt-4">
              <label className="text-sm font-medium">Reason</label>
              <Input
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                placeholder="Recorded as a dossier entry on this controller's log"
              />
              <div className="flex items-center gap-3">
                <Button onClick={onSave} disabled={!valid || save.isPending}>
                  <Save />
                  {save.isPending ? "Saving…" : "Save"}
                </Button>
                {save.isSuccess && (
                  <span className="text-sm text-emerald-600 dark:text-emerald-400">Saved.</span>
                )}
                {save.isError && (
                  <span className="text-sm text-destructive">{saveErrorMessage(save.error)}</span>
                )}
              </div>
            </section>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

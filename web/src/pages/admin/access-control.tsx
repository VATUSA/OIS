import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, cn, Input} from "@ois/ui";
import {ChevronRight, Save, Search} from "lucide-react";

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
import {Pagination} from "@/components/pagination";

type ScopeState = { roles: string[]; perms: string[] };

const USERS_PAGE_SIZE = 25;

function saveErrorMessage(error: unknown): string {
  const status = (error as { status?: number } | null)?.status;
  if (status === 401 || status === 403) {
    return "You can only grant roles and permissions you hold yourself.";
  }
  if (status === 404) return "User not found.";
  return "Save failed.";
}

export function AdminAccessControl() {
  const [cid, setCid] = useState<number | undefined>(undefined);
  const [userQuery, setUserQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [usersPage, setUsersPage] = useState(1);
  const [selected, setSelected] = useState<{ cid: number; name: string } | null>(
    null,
  );

  const catalog = useCatalog();
  const access = useUserAccess(cid);
  const save = useSaveUserAccess();

  const [working, setWorking] = useState<Record<string, ScopeState>>({});
  const [scope, setScope] = useState(""); // "" = national
  const [reason, setReason] = useState("");
  const [search, setSearch] = useState("");
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!access.data) return;
    const next: Record<string, ScopeState> = {};
    for (const s of access.data.scopes) {
      const key = s.artcc_id ?? "";
      next[key] = {
        roles: [...s.role_names],
        perms: flattenTree(s.permissions as PermTree),
      };
    }
    if (!next[""]) next[""] = { roles: [], perms: [] };
    setWorking(next);
    setScope("");
    setReason("");
    save.reset();
  }, [access.data]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(userQuery.trim()), 200);
    return () => clearTimeout(timer);
  }, [userQuery]);
  useEffect(() => setUsersPage(1), [debouncedQuery]);
  const users = useAllUsers(usersPage, USERS_PAGE_SIZE, debouncedQuery);

  const current: ScopeState = working[scope] ?? { roles: [], perms: [] };
  // Server admins hold every permission implicitly — their permission tree is
  // read-only, but their roles are still editable.
  const permsReadOnly = !!access.data?.server_admin;

  const allPerms = useMemo(
    () => flattenTree((catalog.data?.permissions ?? {}) as PermTree).sort(),
    [catalog.data],
  );

  const groups = useMemo(() => {
    const q = search.trim().toLowerCase();
    const map = new Map<string, string[]>();
    for (const perm of allPerms) {
      if (q && !perm.includes(q)) continue;
      const domain = perm.split(".")[0];
      if (!map.has(domain)) map.set(domain, []);
      map.get(domain)!.push(perm);
    }
    return map;
  }, [allPerms, search]);

  function updateScope(mut: (s: ScopeState) => ScopeState) {
    setWorking((w) => ({ ...w, [scope]: mut(w[scope] ?? { roles: [], perms: [] }) }));
  }
  function togglePerm(perm: string) {
    updateScope((s) => ({
      ...s,
      perms: s.perms.includes(perm)
        ? s.perms.filter((p) => p !== perm)
        : [...s.perms, perm],
    }));
  }
  function toggleRole(role: string) {
    updateScope((s) => ({
      ...s,
      roles: s.roles.includes(role)
        ? s.roles.filter((r) => r !== role)
        : [...s.roles, role],
    }));
  }
  function setGroup(perms: string[], on: boolean) {
    updateScope((s) => {
      const set = new Set(s.perms);
      for (const p of perms) (on ? set.add(p) : set.delete(p));
      return { ...s, perms: [...set] };
    });
  }
  function toggleGroupOpen(domain: string) {
    setOpenGroups((s) => {
      const next = new Set(s);
      if (next.has(domain)) next.delete(domain);
      else next.add(domain);
      return next;
    });
  }

  function pickUser(user: AdminUserRow) {
    setSelected({ cid: user.cid, name: user.display_name });
    setCid(user.cid);
  }
  function onSave() {
    if (cid == null || !reason.trim()) return;
    // Only send assignable roles. Non-assignable roles the target may hold (e.g.
    // SERVER_ADMIN) are preserved server-side. For a server admin the permission tree
    // is not editable, so send no permission changes (the server ignores them anyway).
    const assignable = new Set(catalog.data?.roles ?? []);
    const scopes = Object.entries(working).map(([key, val]) => ({
      artcc_id: key === "" ? null : key,
      permissions: (permsReadOnly
        ? {}
        : buildTree(val.perms)) as unknown as Record<string, never>,
      role_names: val.roles.filter((role) => assignable.has(role)),
    }));
    save.mutate({ cid, body: { reason: reason.trim(), scopes } satisfies UpdateBody });
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Access Control</h1>
        <p className="text-muted-foreground">
          Grant roles and fine-grained, per-ARTCC permissions. Every change is
          audited.
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
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <span className="text-lg font-semibold">
                  {selected?.name ?? `CID ${access.data.cid}`}
                </span>
                <span className="text-sm text-muted-foreground">
                  CID {access.data.cid}
                </span>
                {access.data.server_admin && (
                  <Badge variant="success">Server admin</Badge>
                )}
              </div>
              <div className="flex items-center gap-2">
                <label className="text-sm text-muted-foreground">Scope</label>
                <select
                  value={scope}
                  onChange={(e) => {
                    const value = e.target.value;
                    setScope(value);
                    setWorking((w) =>
                      w[value] ? w : { ...w, [value]: { roles: [], perms: [] } },
                    );
                  }}
                  className="h-9 rounded-md border border-input bg-background px-3 text-sm"
                >
                  <option value="">National</option>
                  {catalog.data?.facilities.map((f) => (
                    <option key={f.id} value={f.id}>
                      {f.id} — {f.name}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            {permsReadOnly && (
              <div className="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-sm text-muted-foreground">
                Server admins hold every permission implicitly (managed via{" "}
                <code>OIS_SERVER_ADMIN_CID</code>), so the permission tree is
                read-only. Roles can still be changed.
              </div>
            )}

            {/* Roles */}
            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-semibold">Roles</h3>
              <div className="grid gap-1.5 sm:grid-cols-3 lg:grid-cols-4">
                {catalog.data?.roles.map((role) => (
                  <label
                    key={role}
                    className="flex items-center gap-2 rounded-md px-2 py-1 text-sm hover:bg-accent"
                  >
                    <input
                      type="checkbox"
                      className="size-4 accent-primary"
                      checked={current.roles.includes(role)}
                      onChange={() => toggleRole(role)}
                    />
                    {role}
                  </label>
                ))}
              </div>
            </section>

            {/* Permissions */}
            <section className="flex flex-col gap-2">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold">Direct permissions</h3>
                <div className="relative w-64">
                  <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
                  <Input
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="Search permissions"
                    className="pl-8"
                  />
                </div>
              </div>

              <div className="flex flex-col gap-2">
                {[...groups.entries()].map(([domain, perms]) => {
                  const open = openGroups.has(domain) || search.trim().length > 0;
                  const checked = perms.filter((p) =>
                    current.perms.includes(p),
                  ).length;
                  const allOn = checked === perms.length;
                  return (
                    <div key={domain} className="overflow-hidden rounded-md border">
                      <div className="flex items-center gap-2 bg-muted/40 px-3 py-2">
                        <input
                          type="checkbox"
                          className="size-4 accent-primary"
                          disabled={permsReadOnly}
                          checked={allOn}
                          ref={(el) => {
                            if (el) el.indeterminate = checked > 0 && !allOn;
                          }}
                          onChange={() => setGroup(perms, !allOn)}
                        />
                        <button
                          type="button"
                          onClick={() => toggleGroupOpen(domain)}
                          className="flex flex-1 items-center gap-1 text-left text-sm font-medium"
                        >
                          <ChevronRight
                            className={cn(
                              "size-4 transition-transform",
                              open && "rotate-90",
                            )}
                          />
                          {domain}
                          <span className="ml-auto text-xs font-normal text-muted-foreground">
                            {checked}/{perms.length}
                          </span>
                        </button>
                      </div>
                      {open && (
                        <div className="grid gap-1 border-t px-3 py-2 sm:grid-cols-2">
                          {perms.map((perm) => (
                            <label
                              key={perm}
                              className="flex items-center gap-2 rounded px-1 py-0.5 text-sm hover:bg-accent"
                            >
                              <input
                                type="checkbox"
                                className="size-4 accent-primary"
                                disabled={permsReadOnly}
                                checked={current.perms.includes(perm)}
                                onChange={() => togglePerm(perm)}
                              />
                              <span className="font-mono text-xs">{perm}</span>
                            </label>
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
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
                <Button
                  onClick={onSave}
                  disabled={!reason.trim() || save.isPending}
                >
                  <Save />
                  {save.isPending ? "Saving…" : "Save"}
                </Button>
                {save.isSuccess && (
                  <span className="text-sm text-emerald-600 dark:text-emerald-400">
                    Saved.
                  </span>
                )}
                {save.isError && (
                  <span className="text-sm text-destructive">
                    {saveErrorMessage(save.error)}
                  </span>
                )}
              </div>
            </section>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, cn, Input} from "@ois/ui";
import {ChevronRight, Save, Search} from "lucide-react";

import {
  buildTree,
  flattenTree,
  type PermTree,
  type UpdateBody,
  useCatalog,
  type UserMatch,
  useSaveUserAccess,
  useUserAccess,
  useUserSearch,
} from "@/lib/access";

type ScopeState = { roles: string[]; perms: string[] };

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
  const [selected, setSelected] = useState<{ cid: number; name: string } | null>(
    null,
  );
  const [showResults, setShowResults] = useState(false);

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
  const userSearch = useUserSearch(debouncedQuery);

  const current: ScopeState = working[scope] ?? { roles: [], perms: [] };

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

  function pickUser(user: UserMatch) {
    setSelected({ cid: user.cid, name: user.display_name });
    setCid(user.cid);
    setUserQuery(user.display_name);
    setShowResults(false);
  }
  function onSave() {
    if (cid == null || !reason.trim()) return;
    // Only send assignable roles. Non-assignable roles the target may hold
    // (e.g. SERVER_ADMIN) are not editable here and are preserved server-side.
    const assignable = new Set(catalog.data?.roles ?? []);
    const scopes = Object.entries(working).map(([key, val]) => ({
      artcc_id: key === "" ? null : key,
      permissions: buildTree(val.perms) as unknown as Record<string, never>,
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
        <CardContent className="pt-6">
          <div className="flex flex-col gap-1.5">
            <label className="text-sm font-medium">Find a controller</label>
            <div className="relative w-full max-w-md">
              <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
              <Input
                value={userQuery}
                onChange={(e) => {
                  setUserQuery(e.target.value);
                  setShowResults(true);
                }}
                onFocus={() => setShowResults(true)}
                onBlur={() => setTimeout(() => setShowResults(false), 150)}
                placeholder="Search by name or CID"
                className="pl-8"
              />
              {showResults && debouncedQuery.length >= 1 && (
                <div className="absolute z-30 mt-1 max-h-72 w-full overflow-auto rounded-md border bg-popover shadow-md">
                  {userSearch.isLoading ? (
                    <div className="px-3 py-2 text-sm text-muted-foreground">
                      Searching…
                    </div>
                  ) : (userSearch.data?.length ?? 0) === 0 ? (
                    <div className="px-3 py-2 text-sm text-muted-foreground">
                      No matches.
                    </div>
                  ) : (
                    userSearch.data!.map((user) => (
                      <button
                        key={user.cid}
                        type="button"
                        onClick={() => pickUser(user)}
                        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm hover:bg-accent"
                      >
                        <span className="font-medium">{user.display_name}</span>
                        <span className="text-xs text-muted-foreground">
                          CID {user.cid}
                          {user.rating ? ` · ${user.rating}` : ""}
                        </span>
                      </button>
                    ))
                  )}
                </div>
              )}
            </div>
          </div>
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

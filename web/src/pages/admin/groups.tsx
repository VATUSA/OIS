import {useEffect, useMemo, useState} from "react";
import {Button, Card, ConfirmButton, Input, StatusPill, useToast} from "@ois/ui";
import {Plus, ShieldCheck} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useCatalog, flattenTree} from "@/lib/access";
import {
  type Group,
  useChangeMembership,
  useCreateGroup,
  useDeleteGroup,
  useGroupMembers,
  useGroups,
  useSaveGroup,
} from "@/lib/groups";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";

const SUBTITLE =
  "What each group grants. Editing a group changes every holder's access at once — no per-user re-save.";

const labelClass = "text-xs font-semibold text-ink-2";

/**
 * The permission checkboxes for one group, grouped by domain.
 *
 * Flat rather than the shared `PermissionScopeTree`: `access.role_permissions` carries **no ARTCC
 * scope**. Scope lives on the membership (`access.user_roles.artcc_id`), which is what lets one `EC`
 * group mean "EC at ZDC" for one person and national for another. Rendering a scope control here
 * would imply a dimension the data does not have.
 */
function PermissionList({
  all,
  selected,
  disabled,
  onChange,
}: {
  all: string[];
  selected: Set<string>;
  disabled?: boolean;
  onChange: (next: Set<string>) => void;
}) {
  const byDomain = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const name of all) {
      const domain = name.split(".")[0] ?? "other";
      map.set(domain, [...(map.get(domain) ?? []), name]);
    }
    return [...map.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [all]);

  return (
    <div className="flex flex-col gap-4">
      {byDomain.map(([domain, names]) => (
        <div key={domain} className="flex flex-col gap-1.5">
          <span className={labelClass}>{domain}</span>
          <div className="flex flex-wrap gap-2">
            {names.map((name) => {
              const on = selected.has(name);
              return (
                <label
                  key={name}
                  className={`flex items-center gap-2 rounded-xs border border-line bg-panel-2 px-2.5 py-1 ${
                    disabled ? "opacity-60" : "cursor-pointer"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={disabled}
                    onChange={() => {
                      const next = new Set(selected);
                      if (on) next.delete(name);
                      else next.add(name);
                      onChange(next);
                    }}
                  />
                  <span className="font-mono text-xs text-ink">{name}</span>
                </label>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * Who holds this group, and where.
 *
 * Each scope is its own row because each is its own membership — a user holding `EC` nationally *and*
 * at ZDC has two grants, and showing one row would repeat the flattening the admin user table's badges
 * do. Removal therefore names the scope, not just the person.
 */
function Members({group, facilities}: {group: Group; facilities: {id: string; name: string}[]}) {
  const [page, setPage] = useState(1);
  const members = useGroupMembers(group.name, page);
  const add = useChangeMembership(true);
  const remove = useChangeMembership(false);
  const [cid, setCid] = useState("");
  const [artcc, setArtcc] = useState("");
  const [reason, setReason] = useState("");

  const canAdd = /^\d{5,8}$/.test(cid.trim()) && reason.trim().length > 0;
  const total = members.data?.total ?? 0;
  const pageSize = members.data?.page_size ?? 25;
  const pages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <div className="flex flex-col gap-3 border-t border-line-soft pt-4">
      <span className={labelClass}>Members — {total}</span>

      {members.data?.items.length === 0 && (
        <p className="text-xs text-ink-3">Nobody holds this group.</p>
      )}

      <div className="flex flex-col gap-1">
        {(members.data?.items ?? []).map((m) => (
          <div
            key={`${m.cid}:${m.artcc_id ?? ""}`}
            className="flex items-center justify-between gap-3 rounded-xs bg-panel-2 px-2.5 py-1.5"
          >
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-ink">{m.display_name}</span>
              <span className="font-mono text-xs text-ink-3">{m.cid}</span>
              <StatusPill tone={m.artcc_id ? "brand" : "neutral"}>
                {m.artcc_id ?? "national"}
              </StatusPill>
            </div>
            {!group.system && (
              <ConfirmButton
                variant="ghost"
                onConfirm={() =>
                  remove.mutate({
                    name: group.name,
                    body: {
                      cid: m.cid,
                      artcc_id: m.artcc_id,
                      reason: `Removed from ${group.name}`,
                    },
                  })
                }
              >
                Remove
              </ConfirmButton>
            )}
          </div>
        ))}
      </div>

      {pages > 1 && (
        <div className="flex items-center gap-2">
          <Button variant="outline" disabled={page <= 1} onClick={() => setPage(page - 1)}>
            Previous
          </Button>
          <span className="font-mono text-xs text-ink-3">
            {page} / {pages}
          </span>
          <Button variant="outline" disabled={page >= pages} onClick={() => setPage(page + 1)}>
            Next
          </Button>
        </div>
      )}

      {!group.system && (
        <div className="flex flex-wrap items-end gap-2">
          <label className="flex flex-col gap-1">
            <span className={labelClass}>CID</span>
            <Input value={cid} onChange={(e) => setCid(e.target.value)} placeholder="1234567" />
          </label>
          <label className="flex flex-col gap-1">
            <span className={labelClass}>Scope</span>
            <select
              value={artcc}
              onChange={(e) => setArtcc(e.target.value)}
              className="rounded-xs border border-line bg-panel-2 px-2 py-1.5 text-sm text-ink"
            >
              <option value="">National</option>
              {facilities.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.id}
                </option>
              ))}
            </select>
          </label>
          <label className="flex min-w-48 flex-1 flex-col gap-1">
            <span className={labelClass}>Reason</span>
            <Input value={reason} onChange={(e) => setReason(e.target.value)} />
          </label>
          <Button
            disabled={!canAdd || add.isPending}
            onClick={() =>
              add.mutate(
                {
                  name: group.name,
                  body: {
                    cid: Number(cid.trim()),
                    artcc_id: artcc || undefined,
                    reason: reason.trim(),
                  },
                },
                {onSuccess: () => {
                  setCid("");
                  setReason("");
                }},
              )
            }
          >
            Add member
          </Button>
        </div>
      )}
    </div>
  );
}

function GroupCard({
  group,
  catalog,
  facilities,
}: {
  group: Group;
  catalog: string[];
  facilities: {id: string; name: string}[];
}) {
  const save = useSaveGroup();
  const del = useDeleteGroup();
  const [selected, setSelected] = useState<Set<string>>(() => new Set(group.permissions));
  const [reason, setReason] = useState("");

  useEffect(() => {
    setSelected(new Set(group.permissions));
    setReason("");
  }, [group.permissions]);

  const holders = group.user_count + group.service_account_count;
  const dirty =
    selected.size !== group.permissions.length ||
    group.permissions.some((p) => !selected.has(p));

  return (
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold text-ink">{group.name}</h2>
            {group.system && <StatusPill tone="neutral">system</StatusPill>}
          </div>
          {group.description && <p className="text-sm text-ink-2">{group.description}</p>}
          <p className="font-mono text-xs text-ink-3">
            {group.permissions.length} permissions · {group.user_count} users ·{" "}
            {group.service_account_count} service accounts
          </p>
        </div>
        {!group.system && holders === 0 && (
          <ConfirmButton variant="destructive" onConfirm={() => del.mutate(group.name)}>
            Delete
          </ConfirmButton>
        )}
      </div>

      {group.system ? (
        <p className="text-xs text-ink-3">
          Code depends on this group’s name and contents, so it isn’t editable here.
        </p>
      ) : (
        <>
          <PermissionList all={catalog} selected={selected} onChange={setSelected} />
          <div className="flex flex-wrap items-end gap-2">
            <label className="flex min-w-60 flex-1 flex-col gap-1">
              <span className={labelClass}>Reason (recorded in the audit log)</span>
              <Input value={reason} onChange={(e) => setReason(e.target.value)} />
            </label>
            <Button
              disabled={!dirty || reason.trim().length === 0 || save.isPending}
              onClick={() =>
                save.mutate({
                  name: group.name,
                  body: {permissions: [...selected], reason: reason.trim()},
                })
              }
            >
              Save
            </Button>
          </div>
          {holders > 0 && (
            <p className="text-xs text-warning">
              {holders} principal{holders === 1 ? "" : "s"} hold this group — saving changes their
              access immediately.
            </p>
          )}
        </>
      )}
      <Members group={group} facilities={facilities} />
    </Card>
  );
}

function CreateForm({onDone}: {onDone: () => void}) {
  const create = useCreateGroup();
  const toast = useToast();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [reason, setReason] = useState("");

  const normalised = name.trim().toUpperCase();
  const valid = /^[A-Z_]+$/.test(normalised) && reason.trim().length > 0;

  return (
    <Card className="flex flex-col gap-4 p-5">
      <h2 className="text-xl font-bold text-ink">New group</h2>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Name — capitals and underscores</span>
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="EVENTS_LEAD" />
        </label>
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Description (optional)</span>
          <Input value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
      </div>
      <label className="flex flex-col gap-1">
        <span className={labelClass}>Reason (recorded in the audit log)</span>
        <Input value={reason} onChange={(e) => setReason(e.target.value)} />
      </label>
      <p className="text-xs text-ink-3">
        A new group starts empty. Add its permissions after creating it.
      </p>
      <div className="flex gap-2">
        <Button
          disabled={!valid || create.isPending}
          onClick={() =>
            create.mutate(
              {
                name: normalised,
                description: description.trim() || undefined,
                reason: reason.trim(),
              },
              {
                onSuccess: () => {
                  toast.success(`Created ${normalised}`);
                  onDone();
                },
              },
            )
          }
        >
          Create group
        </Button>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </Card>
  );
}

export function AdminGroups() {
  const {data: me} = useMe();
  const groups = useGroups();
  const catalog = useCatalog();
  const [creating, setCreating] = useState(false);
  const canEdit = hasPermission(me, "access.groups.update");

  const actions = useMemo(
    () =>
      canEdit && !creating ? (
        <Button onClick={() => setCreating(true)}>
          <Plus /> New group
        </Button>
      ) : undefined,
    [canEdit, creating],
  );
  usePageHeader({subtitle: SUBTITLE, count: groups.data?.length ?? null, actions});

  const catalogNames = useMemo(
    () => (catalog.data ? flattenTree(catalog.data.permissions as never).sort() : []),
    [catalog.data],
  );

  if (groups.isLoading) return <p className="text-sm text-ink-3">Loading groups…</p>;
  if (groups.isError)
    return (
      <div className="flex flex-col gap-2">
        <p className="text-sm text-ink-2">Couldn’t load groups.</p>
        <Button variant="outline" onClick={() => groups.refetch()}>
          Retry
        </Button>
      </div>
    );

  return (
    <div className="flex flex-col gap-4">
      {creating && <CreateForm onDone={() => setCreating(false)} />}
      {(groups.data ?? []).map((group) => (
        <GroupCard
          key={group.name}
          group={group}
          catalog={catalogNames}
          facilities={catalog.data?.facilities ?? []}
        />
      ))}
      {!canEdit && (
        <p className="text-xs text-ink-3">
          Editing a group needs <span className="font-mono">access.groups.update</span>.
        </p>
      )}
      {groups.data?.length === 0 && (
        <p className="flex items-center gap-2 text-sm text-ink-3">
          <ShieldCheck className="size-4" /> No groups yet.
        </p>
      )}
    </div>
  );
}

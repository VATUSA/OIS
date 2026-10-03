import {useEffect, useMemo, useState} from "react";
import {Button, Card, ConfirmButton, Input, StatusPill, useToast} from "@ois/ui";
import {Plus, ShieldCheck} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useCatalog, flattenTree} from "@/lib/access";
import {type Group, useCreateGroup, useDeleteGroup, useGroups, useSaveGroup} from "@/lib/groups";
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

function GroupCard({group, catalog}: {group: Group; catalog: string[]}) {
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
        <GroupCard key={group.name} group={group} catalog={catalogNames} />
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

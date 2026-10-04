import {useCallback, useMemo, useState} from "react";
import {
  Button,
  Card,
  type DataColumn,
  DataTable,
  Input,
  Modal,
  Select,
  StatusPill,
  useToast,
} from "@ois/ui";
import {Bot, Check, Clock, Copy, KeyRound, Plus, ShieldCheck, X} from "lucide-react";

import {
  PermissionPicker,
  type PermSelection,
  buildPermissionInputs,
  selectionFromPermissions,
  selectionIsValid,
} from "@/components/api-keys/permission-picker";
import {usePageHeader} from "@/components/shell/page-meta";
import {useFacilities} from "@/lib/admin";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {timeAgo} from "@/lib/time";
import {
  DEFAULT_EXPIRY_DAYS,
  EXPIRY_CHOICES,
  type ServiceAccount,
  type ServiceAccountToken,
  useAssignableRoles,
  useCreateServiceAccount,
  useGrantableServiceAccountPermissions,
  useRotateServiceAccount,
  useServiceAccounts,
  useSetServiceAccountRateLimit,
  useSetServiceAccountPermissions,
  useSetServiceAccountRoles,
  withReveal,
  withoutReveal,
} from "@/lib/service-accounts";
import {usageLabel} from "@/lib/rate-limits";
import {RateLimitCell} from "@/components/credential-rate-limit";

const SUBTITLE =
  "Credentials for machine clients — the Discord bot and tooling. A token is shown once, on creation.";

const labelClass = "text-xs font-semibold text-ink-2";
const shortDate = (iso: string) => new Date(iso).toLocaleDateString();

/**
 * A checkbox per assignable role. The list comes from the service-account roles endpoint, not the
 * access catalog: the catalog serves the human editor's roles and omits BOT, the one role that
 * makes a Discord bot credential work.
 */
function RolePicker({
  roles,
  selected,
  onChange,
}: {
  roles: string[];
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
}) {
  if (roles.length === 0) {
    return <p className="text-sm text-ink-3">No roles available.</p>;
  }
  return (
    <div className="flex flex-wrap gap-2">
      {roles.map((role) => {
        const on = selected.has(role);
        return (
          <label
            key={role}
            className="flex cursor-pointer items-center gap-2 rounded-xs border border-line bg-panel-2 px-3 py-1.5"
          >
            <input
              type="checkbox"
              checked={on}
              onChange={() => {
                const next = new Set(selected);
                if (on) next.delete(role);
                else next.add(role);
                onChange(next);
              }}
            />
            <span className="font-mono text-xs text-ink">{role}</span>
          </label>
        );
      })}
    </div>
  );
}

/** How long a new credential lives. The backend refuses anything past 365 days. */
function ExpirySelect({value, onChange}: {value: number; onChange: (days: number) => void}) {
  return (
    <Select
      aria-label="Token expires in"
      value={String(value)}
      onChange={(e) => onChange(Number(e.target.value))}
    >
      {EXPIRY_CHOICES.map((days) => (
        <option key={days} value={days}>
          {days} days
        </option>
      ))}
    </Select>
  );
}

/** The one-time token reveal — the plaintext is only ever available here. */
function TokenReveal({
  token,
  rolesSet,
  onDismiss,
}: {
  token: ServiceAccountToken;
  rolesSet: boolean;
  onDismiss: () => void;
}) {
  const toast = useToast();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(token.token);
    setCopied(true);
    toast.success("Token copied", {description: "Store it now — it won't be shown again."});
  };
  return (
    <Card className="flex flex-col gap-3 border-brand p-5" role="status">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-xl font-bold text-ink">
            Token for “{token.account.name}”
          </h2>
          <p className="text-sm text-ink-2">
            Copy it now — for your security, it will{" "}
            <strong className="font-bold text-ink">never be shown again</strong>.
          </p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          className="rounded-xs p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
          title="Dismiss"
          aria-label="Dismiss"
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="flex gap-2">
        <Input
          readOnly
          value={token.token}
          className="font-mono text-xs"
          onFocus={(e) => e.target.select()}
        />
        <Button onClick={copy}>
          {copied ? <Check /> : <Copy />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      {rolesSet ? (
        <p className="text-xs text-ink-3">
          Set it as the client’s bearer token:{" "}
          <span className="font-mono text-ink-2">Authorization: Bearer {token.account.key}…</span>
        </p>
      ) : (
        <p className="text-xs text-warning">
          The account was created but its roles were not set — assign them from the table below, or
          it can do nothing.
        </p>
      )}
    </Card>
  );
}

function CreateForm({
  onCreated,
  onCancel,
}: {
  onCreated: (t: ServiceAccountToken, rolesSet: boolean) => void;
  onCancel: () => void;
}) {
  const roles = useAssignableRoles();
  // `mutateAsync`, not `mutate`: roles are a second call, and this flow must know the create
  // succeeded before it can send them.
  const create = useCreateServiceAccount().mutateAsync;
  const setRoles = useSetServiceAccountRoles().mutateAsync;
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [expiresInDays, setExpiresInDays] = useState(DEFAULT_EXPIRY_DAYS);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    let token: ServiceAccountToken;
    try {
      token = await create({
        name: name.trim(),
        description: description.trim() || undefined,
        expires_in_days: expiresInDays,
      });
    } catch {
      setBusy(false);
      return; // the hook already surfaced a toast
    }
    // The token exists and is unrecoverable from here, so it is revealed even if the roles call
    // fails — losing it would mean the account must be rotated to be usable at all.
    let rolesSet = true;
    if (selected.size > 0) {
      try {
        await setRoles({id: token.account.id, body: {role_names: [...selected]}});
      } catch {
        rolesSet = false;
      }
    }
    setBusy(false);
    onCreated(token, rolesSet);
  };

  const canSubmit = name.trim().length > 0 && !busy;

  return (
    <Card className="flex flex-col gap-4 p-5">
      <h2 className="text-xl font-bold text-ink">New service account</h2>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Name</span>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Discord bot"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Description (optional)</span>
          <Input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="What this client is for"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Token expires in</span>
          <ExpirySelect value={expiresInDays} onChange={setExpiresInDays} />
        </label>
      </div>
      <div className="flex flex-col gap-2">
        <span className={labelClass}>
          Roles — or grant specific permissions per ARTCC from the table once created
        </span>
        <RolePicker roles={roles.data ?? []} selected={selected} onChange={setSelected} />
      </div>
      <div className="flex gap-2">
        <Button disabled={!canSubmit} onClick={submit}>
          Create account
        </Button>
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
      </div>
    </Card>
  );
}

/** Change an existing account's roles. The PUT is a full replace, not a patch. */
function EditRoles({account, onDone}: {account: ServiceAccount; onDone: () => void}) {
  const roles = useAssignableRoles();
  const setRoles = useSetServiceAccountRoles();
  const [selected, setSelected] = useState<Set<string>>(() => new Set(account.roles));

  return (
    <Modal
      open
      onClose={onDone}
      title={`Roles · ${account.name}`}
      description={<span className="font-mono">{account.key}</span>}
      size="lg"
      footer={
        <>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            disabled={setRoles.isPending}
            onClick={() =>
              setRoles.mutate(
                {id: account.id, body: {role_names: [...selected]}},
                {onSuccess: onDone},
              )
            }
          >
            Save roles
          </Button>
        </>
      }
    >
      <RolePicker roles={roles.data ?? []} selected={selected} onChange={setSelected} />
    </Modal>
  );
}

/**
 * Grant an account specific permissions at specific ARTCCs. The picker offers only what the signed-in
 * admin holds, at the scope they hold it — the backend refuses anything more. A full replace.
 */
function EditPermissions({account, onDone}: {account: ServiceAccount; onDone: () => void}) {
  const grantable = useGrantableServiceAccountPermissions();
  const facilities = useFacilities();
  const setPermissions = useSetServiceAccountPermissions();
  const [selection, setSelection] = useState<PermSelection>(() =>
    selectionFromPermissions(account.permissions),
  );

  return (
    <Modal
      open
      onClose={onDone}
      title={`Permissions · ${account.name}`}
      description="You can grant only what you hold yourself, at the ARTCCs you hold it."
      size="lg"
      footer={
        <>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            disabled={setPermissions.isPending || !selectionIsValid(selection)}
            onClick={() =>
              setPermissions.mutate(
                {id: account.id, body: {permissions: buildPermissionInputs(selection)}},
                {onSuccess: onDone},
              )
            }
          >
            Save permissions
          </Button>
        </>
      }
    >
      <PermissionPicker
        grantable={grantable.data ?? []}
        facilities={facilities.data ?? []}
        selection={selection}
        onChange={setSelection}
      />
    </Modal>
  );
}

/** Revoke the live token and issue a new one, with a fresh lifetime. */
function RotateToken({
  account,
  onRotated,
  onDone,
}: {
  account: ServiceAccount;
  onRotated: (t: ServiceAccountToken) => void;
  onDone: () => void;
}) {
  const rotate = useRotateServiceAccount();
  const [expiresInDays, setExpiresInDays] = useState(DEFAULT_EXPIRY_DAYS);

  return (
    <Modal
      open
      onClose={onDone}
      title={`Rotate token · ${account.name}`}
      description="The current token stops working immediately. The new one is shown once."
      footer={
        <>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            disabled={rotate.isPending}
            onClick={() =>
              rotate.mutate(
                {id: account.id, expiresInDays},
                {
                  onSuccess: (t) => {
                    onRotated(t);
                    onDone();
                  },
                },
              )
            }
          >
            Rotate token
          </Button>
        </>
      }
    >
      <label className="flex flex-col gap-1">
        <span className={labelClass}>New token expires in</span>
        <ExpirySelect value={expiresInDays} onChange={setExpiresInDays} />
      </label>
    </Modal>
  );
}

export function AdminServiceAccounts() {
  const {data: me} = useMe();
  const accounts = useServiceAccounts();
  const [creating, setCreating] = useState(false);
  const [reveals, setReveals] = useState<ServiceAccountToken[]>([]);
  // Accounts whose create succeeded but whose roles call did not — the token is still real, so the
  // reveal must say so rather than imply a working credential.
  const [rolesFailed, setRolesFailed] = useState<ReadonlySet<string>>(() => new Set());
  const [editing, setEditing] = useState<ServiceAccount | null>(null);
  const [granting, setGranting] = useState<ServiceAccount | null>(null);
  const [rotating, setRotating] = useState<ServiceAccount | null>(null);
  const canCreate = hasPermission(me, "service_accounts.create");
  const canUpdate = hasPermission(me, "service_accounts.update");
  const setLimit = useSetServiceAccountRateLimit();

  const reveal = useCallback((token: ServiceAccountToken, rolesSet: boolean) => {
    setReveals((list) => withReveal(list, token));
    if (!rolesSet) {
      setRolesFailed((ids) => new Set(ids).add(token.account.id));
    }
  }, []);

  const actions = useMemo(
    () =>
      canCreate && !creating ? (
        <Button onClick={() => setCreating(true)}>
          <Plus /> New service account
        </Button>
      ) : undefined,
    [canCreate, creating],
  );
  usePageHeader({subtitle: SUBTITLE, count: accounts.data?.length ?? null, actions});

  const columns = useMemo<DataColumn<ServiceAccount>[]>(
    () => [
      {
        accessorKey: "name",
        header: "Account",
        icon: Bot,
        cell: (c) => {
          const a = c.row.original;
          return (
            <div className="min-w-40">
              <div className="font-semibold">{a.name}</div>
              <div className="font-mono text-xs text-ink-3">{a.key}</div>
              {a.description && <div className="mt-0.5 text-xs text-ink-2">{a.description}</div>}
            </div>
          );
        },
      },
      {
        id: "status",
        header: "Status",
        cell: (c) => (
          <StatusPill tone={toneOf("serviceAccount", c.row.original.status)}>
            {c.row.original.status}
          </StatusPill>
        ),
      },
      {
        id: "roles",
        header: "Roles",
        icon: ShieldCheck,
        cell: (c) => {
          const roles = c.row.original.roles;
          return roles.length === 0 ? (
            <span className="text-xs text-ink-3">none</span>
          ) : (
            <span className="font-mono text-xs text-ink-2">{roles.join(", ")}</span>
          );
        },
      },
      {
        id: "usage",
        header: "Usage",
        mono: true,
        cell: (c) => <span className="whitespace-nowrap text-ink-2">{usageLabel(c.row.original.usage)}</span>,
      },
      {
        id: "rate_limit",
        header: "Limit",
        mono: true,
        cell: (c) => (
          <RateLimitCell
            label={c.row.original.name}
            value={c.row.original.rate_limit_per_min}
            editable={canUpdate}
            onSave={(perMin) => setLimit.mutate({id: c.row.original.id, perMin})}
          />
        ),
        },
        {
        id: "permissions",
        header: "Permissions",
        icon: KeyRound,
        cell: (c) => {
          const perms = c.row.original.permissions;
          return perms.length === 0 ? (
            <span className="text-xs text-ink-3">none</span>
          ) : (
            <span className="font-mono text-xs text-ink-2">
              {perms.map((p) => `${p.permission}@${p.artcc_id ?? "national"}`).join(", ")}
            </span>
          );
        },
      },
      {
        id: "last_used",
        header: "Last used",
        icon: Clock,
        mono: true,
        cell: (c) => (
          <div className="flex items-center gap-2">
            {c.row.original.last_used_at ? timeAgo(c.row.original.last_used_at) : "never"}
            {c.row.original.stale && (
              <StatusPill tone="warn">
                stale
              </StatusPill>
            )}
          </div>
        ),
      },
      {
        id: "expires",
        header: "Expires",
        mono: true,
        cell: (c) => (c.row.original.expires_at ? shortDate(c.row.original.expires_at) : "—"),
      },
      {
        accessorKey: "created_at",
        header: "Created",
        mono: true,
        cell: (c) => shortDate(c.row.original.created_at),
      },
      ...(canUpdate
        ? [
            {
              id: "actions",
              header: "",
              cell: (c) => (
                <div className="flex gap-2">
                  <Button variant="outline" onClick={() => setEditing(c.row.original)}>
                    Roles
                  </Button>
                  <Button variant="outline" onClick={() => setGranting(c.row.original)}>
                    Permissions
                  </Button>
                  {c.row.original.status === "active" && (
                    <Button variant="outline" onClick={() => setRotating(c.row.original)}>
                      Rotate
                    </Button>
                  )}
                </div>
              ),
            } as DataColumn<ServiceAccount>,
          ]
        : []),
    ],
    [canUpdate, setLimit],
  );

  return (
    <div className="flex flex-col gap-4">
      {reveals.map((t) => (
        <TokenReveal
          key={t.account.id}
          token={t}
          rolesSet={!rolesFailed.has(t.account.id)}
          onDismiss={() => setReveals((list) => withoutReveal(list, t.account.id))}
        />
      ))}

      {creating && (
        <CreateForm
          onCreated={(t, rolesSet) => {
            reveal(t, rolesSet);
            setCreating(false);
          }}
          onCancel={() => setCreating(false)}
        />
      )}

      <DataTable
        label="Service accounts"
        columns={columns}
        data={accounts.data ?? []}
        getRowId={(a) => a.id}
        rowCap={25}
        isLoading={accounts.isLoading}
        isError={accounts.isError}
        onRetry={() => accounts.refetch()}
        empty="No service accounts yet."
      />

      {!canCreate && (
        <p className="text-xs text-ink-3">
          Creating an account needs <span className="font-mono">service_accounts.create</span>.
        </p>
      )}

      {editing && <EditRoles account={editing} onDone={() => setEditing(null)} />}
      {granting && <EditPermissions account={granting} onDone={() => setGranting(null)} />}
      {rotating && (
        <RotateToken
          account={rotating}
          onRotated={(t) => reveal(t, true)}
          onDone={() => setRotating(null)}
        />
      )}
    </div>
  );
}

import {useCallback, useMemo, useState} from "react";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  EmptyState,
  Input,
  Modal,
  QueryState,
  StatusPill,
  useToast,
} from "@ois/ui";
import {Activity, CalendarClock, Check, Clock, Copy, KeyRound, Lock, Plus, ShieldCheck, X} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useFacilities} from "@/lib/admin";
import {toneOf} from "@/lib/status";
import {timeAgo} from "@/lib/time";
import {
  type ApiKey,
  type ApiKeyToken,
  useCreateKey,
  useDeleteKey,
  useDisableKey,
  useGrantablePermissions,
  useKeyAudit,
  useMyKeys,
  useRotateKey,
  useSetKeyPermissions,
  withoutReveal,
  withReveal,
} from "@/lib/api-keys";
import {
  buildPermissionInputs,
  PermissionPicker,
  type PermSelection,
  selectionFromPermissions,
  selectionIsValid,
} from "@/components/api-keys/permission-picker";

const SUBTITLE =
  "Personal access tokens for integrating with the OIS API. A key can never do more than you can — it's capped by your live permissions.";

/** The key's displayed state: disabled wins, then expired (derived from `expires_at`), else active. */
function keyState(k: ApiKey): "active" | "disabled" | "expired" {
  if (k.status !== "active") return "disabled";
  if (k.expires_at != null && new Date(k.expires_at).getTime() < Date.now()) return "expired";
  return "active";
}

/** A key's state as a status pill. */
export function StatusBadge({ k }: { k: ApiKey }) {
  const s = keyState(k);
  return <StatusPill tone={toneOf("apiKey", s)}>{s}</StatusPill>;
}

/** The one-time token reveal — the plaintext is only ever available here. */
function TokenReveal({ token, onDismiss }: { token: ApiKeyToken; onDismiss: () => void }) {
  const toast = useToast();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(token.token);
    setCopied(true);
    toast.success("Token copied", { description: "Store it now — it won't be shown again." });
  };
  return (
    <Card className="flex flex-col gap-3 border-brand p-5" role="status">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-xl font-bold text-ink">Your new token for “{token.key.name}”</h2>
          <p className="text-sm text-ink-2">
            Copy it now — for your security, it will <strong className="font-bold text-ink">never be shown again</strong>.
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
        <Input readOnly value={token.token} className="font-mono text-xs" onFocus={(e) => e.target.select()} />
        <Button onClick={copy}>
          {copied ? <Check /> : <Copy />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <p className="text-xs text-ink-3">
        Use it as a bearer token:{" "}
        <span className="font-mono text-ink-2">Authorization: Bearer {token.key.prefix}…</span>
      </p>
    </Card>
  );
}

/** A key's audit activity, a page of the API at a time. Shared with the admin keys page. */
export function KeyActivity({ id }: { id: string }) {
  const [page, setPage] = useState(1);
  const audit = useKeyAudit(id, page);
  type Entry = NonNullable<typeof audit.data>["items"][number];
  const columns = useMemo<DataColumn<Entry>[]>(
    () => [
      {
        accessorKey: "created_at",
        header: "When",
        icon: Clock,
        mono: true,
        cell: (c) => <span className="whitespace-nowrap text-ink-2">{timeAgo(c.getValue<string>())}</span>,
      },
      {
        accessorKey: "action",
        header: "Action",
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      {
        id: "resource",
        accessorFn: (e) => `${e.resource_type}${e.resource_id ? ` ${e.resource_id}` : ""}`,
        header: "Resource",
        cell: (c) => <span className="text-ink-2">{c.getValue<string>()}</span>,
      },
    ],
    [],
  );
  return (
    <DataTable
      label="Key activity"
      columns={columns}
      data={audit.data?.items ?? []}
      getRowId={(e) => String(e.id)}
      rowCap={50}
      isLoading={audit.isLoading}
      isError={audit.isError}
      onRetry={() => audit.refetch()}
      empty="No recorded activity yet."
      serverPagination={
        audit.data && {
          page,
          pageSize: audit.data.page_size ?? 50,
          total: audit.data.total ?? 0,
          onPageChange: setPage,
        }
      }
    />
  );
}

function EditPermissions({ k, onDone }: { k: ApiKey; onDone: () => void }) {
  const grantable = useGrantablePermissions();
  const facilities = useFacilities();
  const setPerms = useSetKeyPermissions();
  const [selection, setSelection] = useState<PermSelection>(() => selectionFromPermissions(k.permissions));

  const save = () => {
    setPerms.mutate({ id: k.id, body: { permissions: buildPermissionInputs(selection) } }, { onSuccess: onDone });
  };

  return (
    <Modal
      open
      onClose={onDone}
      title={`Permissions · ${k.name}`}
      description={<span className="font-mono">{k.prefix}…</span>}
      size="xl"
      footer={
        <>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button disabled={!selectionIsValid(selection) || setPerms.isPending} onClick={save}>
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

function CreateForm({ onCreated, onCancel }: { onCreated: (t: ApiKeyToken) => void; onCancel: () => void }) {
  const grantable = useGrantablePermissions();
  const facilities = useFacilities();
  const create = useCreateKey();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [selection, setSelection] = useState<PermSelection>(() => new Map());

  const submit = () => {
    create.mutate(
      {
        name: name.trim(),
        description: description.trim() || undefined,
        expires_at: expiresAt ? new Date(expiresAt).toISOString() : undefined,
        permissions: buildPermissionInputs(selection),
      },
      {
        onSuccess: (t) => {
          onCreated(t);
        },
      },
    );
  };

  const canSubmit = name.trim().length > 0 && selectionIsValid(selection) && !create.isPending;
  const labelClass = "text-xs font-semibold text-ink-2";

  return (
    <Card className="flex flex-col gap-4 p-5">
      <h2 className="text-xl font-bold text-ink">New API key</h2>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Name</span>
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="My integration" />
        </label>
        <label className="flex flex-col gap-1">
          <span className={labelClass}>Expires (optional)</span>
          <Input type="datetime-local" value={expiresAt} onChange={(e) => setExpiresAt(e.target.value)} />
        </label>
      </div>
      <label className="flex flex-col gap-1">
        <span className={labelClass}>Description (optional)</span>
        <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What this key is for" />
      </label>
      <div className="flex flex-col gap-1">
        <span className={labelClass}>Permissions — a key can only be granted what you currently hold</span>
        <PermissionPicker
          grantable={grantable.data ?? []}
          facilities={facilities.data ?? []}
          selection={selection}
          onChange={setSelection}
        />
      </div>
      <div className="flex gap-2">
        <Button disabled={!canSubmit} onClick={submit}>
          Create key
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </Card>
  );
}

export function ApiKeysPage() {
  const { data: me, isLoading } = useMe();
  const keys = useMyKeys();
  // `mutateAsync`, not `mutate`: a per-call `onSuccess` fires only for the latest `mutate`, which would
  // drop the token of an earlier rotation still in flight.
  const rotate = useRotateKey().mutateAsync;
  const disable = useDisableKey().mutate;
  const del = useDeleteKey().mutate;
  const [creating, setCreating] = useState(false);
  const [reveals, setReveals] = useState<ApiKeyToken[]>([]);
  const reveal = useCallback((t: ApiKeyToken) => setReveals((list) => withReveal(list, t)), []);
  const [activityFor, setActivityFor] = useState<ApiKey | null>(null);
  const [editing, setEditing] = useState<ApiKey | null>(null);
  const canCreate = hasPermission(me, "api_keys.key.create");

  const actions = useMemo(
    () =>
      canCreate && !creating ? (
        <Button onClick={() => setCreating(true)}>
          <Plus /> New key
        </Button>
      ) : undefined,
    [canCreate, creating],
  );
  usePageHeader({ subtitle: SUBTITLE, count: canCreate ? (keys.data?.length ?? null) : null, actions });

  const columns = useMemo<DataColumn<ApiKey>[]>(
    () => [
      {
        accessorKey: "name",
        header: "Key",
        icon: KeyRound,
        cell: (c) => {
          const k = c.row.original;
          return (
            <div className="min-w-40">
              <div className="font-semibold">{k.name}</div>
              <div className="font-mono text-xs text-ink-3">{k.prefix}…</div>
              {k.description && <div className="mt-0.5 text-xs text-ink-2">{k.description}</div>}
            </div>
          );
        },
      },
      {
        id: "status",
        accessorFn: keyState,
        header: "Status",
        cell: (c) => <StatusBadge k={c.row.original} />,
      },
      {
        id: "permissions",
        accessorFn: (k) => k.permissions.length,
        header: "Permissions",
        icon: ShieldCheck,
        cell: (c) => {
          const perms = c.row.original.permissions;
          if (perms.length === 0) return <span className="text-xs text-ink-3">no permissions</span>;
          return (
            <div className="flex max-w-md flex-wrap gap-1">
              {perms.slice(0, 6).map((p, i) => (
                <StatusPill key={i} className="font-mono font-normal">
                  {p.permission}
                  {p.artcc_id ? `@${p.artcc_id}` : ""}
                </StatusPill>
              ))}
              {perms.length > 6 && (
                <span className="self-center font-mono text-xs text-ink-3">+{perms.length - 6} more</span>
              )}
            </div>
          );
        },
      },
      {
        accessorKey: "last_used_at",
        header: "Last used",
        icon: Clock,
        mono: true,
        cell: (c) => {
          const v = c.getValue<string | null>();
          return <span className="whitespace-nowrap text-ink-2">{v ? timeAgo(v) : "never"}</span>;
        },
      },
      {
        accessorKey: "expires_at",
        header: "Expires",
        icon: CalendarClock,
        mono: true,
        cell: (c) => {
          const v = c.getValue<string | null>();
          return <span className="whitespace-nowrap text-ink-2">{v ? new Date(v).toLocaleDateString() : "—"}</span>;
        },
      },
      {
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => {
          const k = c.row.original;
          return (
            <div className="flex justify-end gap-1.5">
              <Button size="sm" variant="ghost" onClick={() => setActivityFor(k)}>
                <Activity />
                Activity
              </Button>
              {k.status === "active" && (
                <Button size="sm" variant="outline" onClick={() => setEditing(k)}>
                  Permissions
                </Button>
              )}
              {k.status === "active" && (
                <ConfirmButton
                  size="sm"
                  variant="outline"
                  warn="Rotate the secret? The current token stops working."
                  onConfirm={() => rotate(k.id).then(reveal, () => {})}
                >
                  Rotate
                </ConfirmButton>
              )}
              {k.status === "active" && (
                <ConfirmButton
                  size="sm"
                  variant="outline"
                  warn="Disable this key? It stops working immediately."
                  onConfirm={() => disable(k.id)}
                >
                  Disable
                </ConfirmButton>
              )}
              <ConfirmButton
                size="sm"
                variant="destructive"
                warn="Delete this key permanently?"
                onConfirm={() => del(k.id)}
              >
                Delete
              </ConfirmButton>
            </div>
          );
        },
      },
    ],
    [rotate, reveal, disable, del],
  );

  if (isLoading) return <QueryState isLoading />;
  if (!canCreate) {
    return (
      <Card>
        <EmptyState icon={Lock} title="No access">
          You don&apos;t have permission to create API keys. Ask an administrator to grant you
          <span className="font-mono"> api_keys.key.create</span>.
        </EmptyState>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {reveals.map((t) => (
        <TokenReveal key={t.key.id} token={t} onDismiss={() => setReveals((list) => withoutReveal(list, t.key.id))} />
      ))}

      {creating && (
        <CreateForm
          onCreated={(t) => {
            reveal(t);
            setCreating(false);
          }}
          onCancel={() => setCreating(false)}
        />
      )}

      <DataTable
        label="Your API keys"
        columns={columns}
        data={keys.data ?? []}
        getRowId={(k) => k.id}
        rowCap={25}
        isLoading={!keys.data && !keys.isError}
        isError={keys.isError}
        onRetry={() => keys.refetch()}
        empty="No API keys yet. Create one to start integrating."
      />

      <Modal
        open={activityFor != null}
        onClose={() => setActivityFor(null)}
        title={activityFor ? `Activity · ${activityFor.name}` : undefined}
        description={activityFor ? <span className="font-mono">{activityFor.prefix}…</span> : undefined}
        placement="right"
      >
        {activityFor && <KeyActivity id={activityFor.id} />}
      </Modal>

      {editing && <EditPermissions k={editing} onDone={() => setEditing(null)} />}
    </div>
  );
}

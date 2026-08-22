import {useState} from "react";
import {Badge, Button, ConfirmButton, Card, CardContent, Input, useToast} from "@ois/ui";
import {Check, Copy, KeyRound, Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useFacilities} from "@/lib/admin";
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
} from "@/lib/api-keys";
import {
  buildPermissionInputs,
  PermissionPicker,
  type PermSelection,
  selectionFromPermissions,
  selectionIsValid,
} from "@/components/api-keys/permission-picker";

/** ISO string ↔ the value of a <input type="datetime-local">. */
function toLocalInput(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function StatusBadge({ k }: { k: ApiKey }) {
  const expired = k.expires_at != null && new Date(k.expires_at).getTime() < Date.now();
  if (k.status !== "active") return <Badge variant="secondary">disabled</Badge>;
  if (expired) return <Badge variant="destructive">expired</Badge>;
  return <Badge variant="success">active</Badge>;
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
    <Card className="border-primary/50">
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="font-medium">Your new token for “{token.key.name}”</div>
            <p className="text-sm text-muted-foreground">
              Copy it now — for your security, it will <strong>never be shown again</strong>.
            </p>
          </div>
          <button type="button" onClick={onDismiss} className="text-muted-foreground hover:text-foreground" title="Dismiss">
            <X className="size-4" />
          </button>
        </div>
        <div className="flex gap-2">
          <Input readOnly value={token.token} className="font-mono text-xs" onFocus={(e) => e.target.select()} />
          <Button variant="secondary" onClick={copy}>
            {copied ? <Check className="mr-1 size-4" /> : <Copy className="mr-1 size-4" />}
            {copied ? "Copied" : "Copy"}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Use it as a bearer token: <span className="font-mono">Authorization: Bearer {token.key.prefix}…</span>
        </p>
      </CardContent>
    </Card>
  );
}

function PermSummary({ k }: { k: ApiKey }) {
  if (k.permissions.length === 0) {
    return <span className="text-xs text-muted-foreground">no permissions</span>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {k.permissions.slice(0, 6).map((p, i) => (
        <span key={i} className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
          {p.permission}
          {p.artcc_id ? `@${p.artcc_id}` : ""}
        </span>
      ))}
      {k.permissions.length > 6 && (
        <span className="text-[10px] text-muted-foreground">+{k.permissions.length - 6} more</span>
      )}
    </div>
  );
}

export function KeyActivity({ id }: { id: string }) {
  const [page, setPage] = useState(1);
  const audit = useKeyAudit(id, page);
  if (audit.isLoading) return <p className="text-xs text-muted-foreground">Loading activity…</p>;
  const items = audit.data?.items ?? [];
  if (items.length === 0) return <p className="text-xs text-muted-foreground">No recorded activity yet.</p>;
  const total = audit.data?.total ?? 0;
  const pages = Math.max(1, Math.ceil(total / (audit.data?.page_size ?? 50)));
  return (
    <div className="flex flex-col gap-1">
      <ul className="flex flex-col divide-y">
        {items.map((e) => (
          <li key={e.id} className="flex flex-wrap items-baseline gap-x-2 py-1 text-xs">
            <span className="font-mono text-muted-foreground">{timeAgo(e.created_at)}</span>
            <span className="font-medium">{e.action}</span>
            <span className="text-muted-foreground">
              {e.resource_type}
              {e.resource_id ? ` ${e.resource_id}` : ""}
            </span>
          </li>
        ))}
      </ul>
      {pages > 1 && (
        <div className="flex items-center gap-2 text-xs">
          <Button size="sm" variant="ghost" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
            Prev
          </Button>
          <span className="text-muted-foreground">
            {page} / {pages}
          </span>
          <Button size="sm" variant="ghost" disabled={page >= pages} onClick={() => setPage((p) => p + 1)}>
            Next
          </Button>
        </div>
      )}
    </div>
  );
}

function EditPermissions({ k, onDone }: { k: ApiKey; onDone: () => void }) {
  const grantable = useGrantablePermissions();
  const facilities = useFacilities();
  const setPerms = useSetKeyPermissions();
  const [selection, setSelection] = useState<PermSelection>(() => selectionFromPermissions(k.permissions));

  const save = () => {
    setPerms.mutate(
      { id: k.id, body: { permissions: buildPermissionInputs(selection) } },
      { onSuccess: onDone },
    );
  };

  return (
    <div className="mt-2 flex flex-col gap-2 rounded-md border bg-muted/20 p-3">
      <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Edit permissions</div>
      <PermissionPicker
        grantable={grantable.data ?? []}
        facilities={facilities.data ?? []}
        selection={selection}
        onChange={setSelection}
      />
      <div className="flex gap-2">
        <Button
          size="sm"
          disabled={!selectionIsValid(selection) || setPerms.isPending}
          onClick={save}
        >
          Save permissions
        </Button>
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

function KeyCard({ k }: { k: ApiKey }) {
  const rotate = useRotateKey();
  const disable = useDisableKey();
  const del = useDeleteKey();
  const [panel, setPanel] = useState<"activity" | "edit" | null>(null);
  const [rotated, setRotated] = useState<ApiKeyToken | null>(null);

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <KeyRound className="size-4 text-muted-foreground" />
              <span className="font-medium">{k.name}</span>
              <StatusBadge k={k} />
            </div>
            <p className="mt-0.5 font-mono text-xs text-muted-foreground">{k.prefix}…</p>
            {k.description && <p className="mt-1 text-sm text-muted-foreground">{k.description}</p>}
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => setPanel(panel === "activity" ? null : "activity")}>
              Activity
            </Button>
            {k.status === "active" && (
              <Button size="sm" variant="outline" onClick={() => setPanel(panel === "edit" ? null : "edit")}>
                Permissions
              </Button>
            )}
            {k.status === "active" && (
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Rotate the secret? The current token stops working."
                onConfirm={() => rotate.mutate(k.id, { onSuccess: (t) => setRotated(t) })}
              >
                Rotate
              </ConfirmButton>
            )}
            {k.status === "active" && (
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Disable this key? It stops working immediately."
                onConfirm={() => disable.mutate(k.id)}
              >
                Disable
              </ConfirmButton>
            )}
            <ConfirmButton
              size="sm"
              variant="destructive"
              warn="Delete this key permanently?"
              onConfirm={() => del.mutate(k.id)}
            >
              Delete
            </ConfirmButton>
          </div>
        </div>

        <PermSummary k={k} />

        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span>
            {k.permissions.length} permission{k.permissions.length === 1 ? "" : "s"}
          </span>
          {k.expires_at && <span>expires {new Date(k.expires_at).toLocaleDateString()}</span>}
          <span>{k.last_used_at ? `last used ${timeAgo(k.last_used_at)}` : "never used"}</span>
        </div>

        {rotated && <TokenReveal token={rotated} onDismiss={() => setRotated(null)} />}
        {panel === "activity" && <KeyActivity id={k.id} />}
        {panel === "edit" && <EditPermissions k={k} onDone={() => setPanel(null)} />}
      </CardContent>
    </Card>
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

  return (
    <Card className="border-primary/40">
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="text-sm font-semibold">New API key</div>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-muted-foreground">Name</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="My integration" />
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-muted-foreground">Expires (optional)</label>
            <Input type="datetime-local" value={expiresAt} onChange={(e) => setExpiresAt(e.target.value)} />
          </div>
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Description (optional)</label>
          <Input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="What this key is for"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">
            Permissions — a key can only be granted what you currently hold
          </label>
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
      </CardContent>
    </Card>
  );
}

export function ApiKeysPage() {
  const { data: me, isLoading } = useMe();
  const keys = useMyKeys();
  const [creating, setCreating] = useState(false);
  const [revealed, setRevealed] = useState<ApiKeyToken | null>(null);

  if (isLoading) {
    return <p className="py-10 text-center text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasPermission(me, "api_keys.key.create")) {
    return (
      <div className="mx-auto w-full max-w-3xl">
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            You don&apos;t have permission to create API keys. Ask an administrator to grant you
            <span className="font-mono"> api_keys.key.create</span>.
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">API keys</h1>
          <p className="text-muted-foreground">
            Personal access tokens for integrating with the OIS API. A key can never do more than you
            can — it&apos;s capped by your live permissions.
          </p>
        </div>
        {!creating && (
          <Button onClick={() => setCreating(true)}>
            <Plus className="mr-1 size-4" /> New key
          </Button>
        )}
      </div>

      {revealed && <TokenReveal token={revealed} onDismiss={() => setRevealed(null)} />}

      {creating && (
        <CreateForm
          onCreated={(t) => {
            setRevealed(t);
            setCreating(false);
          }}
          onCancel={() => setCreating(false)}
        />
      )}

      {keys.isError ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            Couldn&apos;t load your keys.
          </CardContent>
        </Card>
      ) : !keys.data ? (
        <p className="py-6 text-center text-sm text-muted-foreground">Loading keys…</p>
      ) : keys.data.length === 0 ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            No API keys yet. Create one to start integrating.
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-3">
          {keys.data.map((k) => (
            <KeyCard key={k.id} k={k} />
          ))}
        </div>
      )}
    </div>
  );
}

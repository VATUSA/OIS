import {useState} from "react";
import {Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {KeyRound} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {timeAgo} from "@/lib/time";
import {type ApiKey, useAdminDeleteKey, useAdminDisableKey, useAllKeys} from "@/lib/api-keys";
import {KeyActivity, StatusBadge} from "@/pages/api-keys";

function AdminKeyCard({ k, canRevoke }: { k: ApiKey; canRevoke: boolean }) {
  const disable = useAdminDisableKey();
  const del = useAdminDeleteKey();
  const [showActivity, setShowActivity] = useState(false);

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
            <p className="mt-0.5 text-xs text-muted-foreground">
              <span className="font-mono">{k.prefix}…</span> · owner{" "}
              {k.owner_display_name ?? "?"}
              {k.owner_cid ? ` (CID ${k.owner_cid})` : ""}
            </p>
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => setShowActivity((v) => !v)}>
              Activity
            </Button>
            {canRevoke && k.status === "active" && (
              <ConfirmButton
                size="sm"
                variant="outline"
                warn="Disable this key? It stops working immediately."
                onConfirm={() => disable.mutate({ id: k.id })}
              >
                Disable
              </ConfirmButton>
            )}
            {canRevoke && (
              <ConfirmButton
                size="sm"
                variant="destructive"
                warn="Delete this key permanently?"
                onConfirm={() => del.mutate(k.id)}
              >
                Delete
              </ConfirmButton>
            )}
          </div>
        </div>

        <div className="flex flex-wrap gap-1">
          {k.permissions.slice(0, 8).map((p, i) => (
            <span key={i} className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
              {p.permission}
              {p.artcc_id ? `@${p.artcc_id}` : ""}
            </span>
          ))}
          {k.permissions.length > 8 && (
            <span className="text-[10px] text-muted-foreground">+{k.permissions.length - 8} more</span>
          )}
          {k.permissions.length === 0 && (
            <span className="text-xs text-muted-foreground">no permissions</span>
          )}
        </div>

        <div className="flex flex-wrap gap-x-4 text-xs text-muted-foreground">
          {k.expires_at && <span>expires {new Date(k.expires_at).toLocaleDateString()}</span>}
          <span>{k.last_used_at ? `last used ${timeAgo(k.last_used_at)}` : "never used"}</span>
          <span>created {new Date(k.created_at).toLocaleDateString()}</span>
        </div>

        {showActivity && <KeyActivity id={k.id} />}
      </CardContent>
    </Card>
  );
}

export function AdminApiKeys() {
  const { data: me } = useMe();
  const canRevoke = hasPermission(me, "api_keys.key.delete");
  const [cidInput, setCidInput] = useState("");
  const [ownerCid, setOwnerCid] = useState<number | undefined>(undefined);
  const keys = useAllKeys(ownerCid);

  const applyFilter = () => {
    const n = Number(cidInput.trim());
    setOwnerCid(cidInput.trim() && Number.isFinite(n) ? n : undefined);
  };

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">API Keys</h1>
        <p className="text-muted-foreground">
          Every user-owned API key across the platform. Keys are always capped by their owner&apos;s
          live permissions.
        </p>
      </div>

      <div className="flex items-end gap-2">
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Filter by owner CID</label>
          <Input
            value={cidInput}
            onChange={(e) => setCidInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && applyFilter()}
            placeholder="1234567"
            className="h-9 w-40 font-mono"
          />
        </div>
        <Button variant="outline" onClick={applyFilter}>
          Filter
        </Button>
        {ownerCid != null && (
          <Button
            variant="ghost"
            onClick={() => {
              setCidInput("");
              setOwnerCid(undefined);
            }}
          >
            Clear
          </Button>
        )}
        <span className="ml-auto self-center text-xs text-muted-foreground">
          {keys.data ? `${keys.data.length} key${keys.data.length === 1 ? "" : "s"}` : ""}
        </span>
      </div>

      {keys.isError ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            Couldn&apos;t load API keys.
          </CardContent>
        </Card>
      ) : !keys.data ? (
        <p className="py-6 text-center text-sm text-muted-foreground">Loading…</p>
      ) : keys.data.length === 0 ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            {ownerCid != null ? "No keys for that CID." : "No API keys yet."}
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-3">
          {keys.data.map((k) => (
            <AdminKeyCard key={k.id} k={k} canRevoke={canRevoke} />
          ))}
        </div>
      )}

      {!canRevoke && (
        <p className="text-xs text-muted-foreground">
          You can view keys but need <span className="font-mono">api_keys.key.delete</span> to revoke
          them.
        </p>
      )}
    </div>
  );
}

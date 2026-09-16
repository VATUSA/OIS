import {useMemo, useState} from "react";
import {
  Button,
  ConfirmButton,
  type DataColumn,
  DataTable,
  FilterBar,
  Input,
  Modal,
  StatusPill,
} from "@ois/ui";
import {Activity, CalendarClock, Clock, KeyRound, ShieldCheck, User} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {timeAgo} from "@/lib/time";
import {type ApiKey, useAdminDeleteKey, useAdminDisableKey, useAllKeys} from "@/lib/api-keys";
import {KeyActivity} from "@/pages/api-keys";

const SUBTITLE =
  "Every user-owned API key across the platform. Keys are always capped by their owner's live permissions.";

/** The key's displayed state: disabled wins, then expired (derived from `expires_at`), else active. */
function keyState(k: ApiKey): "active" | "disabled" | "expired" {
  if (k.status !== "active") return "disabled";
  if (k.expires_at != null && new Date(k.expires_at).getTime() < Date.now()) return "expired";
  return "active";
}

const shortDate = (iso: string) => new Date(iso).toLocaleDateString();

export function AdminApiKeys() {
  const { data: me } = useMe();
  const canRevoke = hasPermission(me, "api_keys.key.delete");
  const [cidInput, setCidInput] = useState("");
  const [ownerCid, setOwnerCid] = useState<number | undefined>(undefined);
  const [activityFor, setActivityFor] = useState<ApiKey | null>(null);
  const keys = useAllKeys(ownerCid);
  const disableKey = useAdminDisableKey().mutate;
  const deleteKey = useAdminDeleteKey().mutate;

  usePageHeader({ subtitle: SUBTITLE, count: keys.data?.length ?? null });

  const applyFilter = () => {
    const n = Number(cidInput.trim());
    setOwnerCid(cidInput.trim() && Number.isFinite(n) ? n : undefined);
  };

  const columns = useMemo<DataColumn<ApiKey>[]>(
    () => [
      {
        accessorKey: "name",
        header: "Key",
        icon: KeyRound,
        cell: (c) => (
          <div className="whitespace-nowrap">
            <div className="font-semibold">{c.row.original.name}</div>
            <div className="font-mono text-xs text-ink-3">{c.row.original.prefix}…</div>
          </div>
        ),
      },
      {
        id: "owner",
        accessorFn: (k) => k.owner_display_name ?? "?",
        header: "Owner",
        icon: User,
        cell: (c) => (
          <span className="whitespace-nowrap">
            <span className="font-semibold">{c.getValue<string>()}</span>
            {c.row.original.owner_cid != null && (
              <span className="ml-1.5 font-mono text-xs text-ink-3">{c.row.original.owner_cid}</span>
            )}
          </span>
        ),
      },
      {
        id: "status",
        accessorFn: keyState,
        header: "Status",
        cell: (c) => {
          const s = c.getValue<string>();
          return <StatusPill tone={toneOf("apiKey", s)}>{s}</StatusPill>;
        },
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
              {perms.slice(0, 8).map((p, i) => (
                <StatusPill key={i} tone="neutral" className="font-mono font-normal">
                  {p.permission}
                  {p.artcc_id ? `@${p.artcc_id}` : ""}
                </StatusPill>
              ))}
              {perms.length > 8 && (
                <span className="self-center font-mono text-xs text-ink-3">+{perms.length - 8} more</span>
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
          return <span className="whitespace-nowrap text-ink-2">{v ? shortDate(v) : "—"}</span>;
        },
      },
      {
        accessorKey: "created_at",
        header: "Created",
        mono: true,
        cell: (c) => <span className="whitespace-nowrap text-ink-2">{shortDate(c.getValue<string>())}</span>,
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
              {canRevoke && k.status === "active" && (
                <ConfirmButton
                  size="sm"
                  variant="outline"
                  warn="Disable this key? It stops working immediately."
                  onConfirm={() => disableKey({ id: k.id })}
                >
                  Disable
                </ConfirmButton>
              )}
              {canRevoke && (
                <ConfirmButton
                  size="sm"
                  variant="destructive"
                  warn="Delete this key permanently?"
                  onConfirm={() => deleteKey(k.id)}
                >
                  Delete
                </ConfirmButton>
              )}
            </div>
          );
        },
      },
    ],
    [canRevoke, disableKey, deleteKey],
  );

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <Input
          aria-label="Filter by owner CID"
          value={cidInput}
          onChange={(e) => setCidInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && applyFilter()}
          placeholder="Owner CID"
          className="h-8 w-40 rounded-full font-mono"
        />
        <Button size="sm" variant="outline" onClick={applyFilter}>
          Filter
        </Button>
        {ownerCid != null && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setCidInput("");
              setOwnerCid(undefined);
            }}
          >
            Clear
          </Button>
        )}
      </FilterBar>

      <DataTable
        label="API keys"
        columns={columns}
        data={keys.data ?? []}
        getRowId={(k) => k.id}
        rowCap={25}
        isLoading={keys.isLoading}
        isError={keys.isError}
        onRetry={() => keys.refetch()}
        empty={ownerCid != null ? "No keys for that CID." : "No API keys yet."}
      />

      {!canRevoke && (
        <p className="text-xs text-ink-3">
          You can view keys but need <span className="font-mono">api_keys.key.delete</span> to revoke
          them.
        </p>
      )}

      <Modal
        open={activityFor != null}
        onClose={() => setActivityFor(null)}
        title={activityFor ? `Activity · ${activityFor.name}` : undefined}
        description={activityFor ? <span className="font-mono">{activityFor.prefix}…</span> : undefined}
        placement="right"
      >
        {activityFor && <KeyActivity id={activityFor.id} />}
      </Modal>
    </div>
  );
}

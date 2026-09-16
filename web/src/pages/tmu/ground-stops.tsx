import {useMemo, useState} from "react";
import {Button, ConfirmButton, type DataColumn, DataTable, Input, StatusPill, useToast} from "@ois/ui";
import {CircleDot, Clock, Plane, Plus, Radar} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {formatZulu} from "@/lib/time";
import {
  type CreateGroundStop,
  type GroundStop,
  useCancelGroundStop,
  useCreateGroundStop,
  useDeleteGroundStop,
  useGroundStops,
  usePublishGroundStop,
} from "@/lib/tmu";

const LABEL = "flex flex-col gap-1 text-xs font-semibold text-ink-2";

const EMPTY: CreateGroundStop = { airport: "", scope: "", until: "" };

function CreateForm() {
  const create = useCreateGroundStop();
  const toast = useToast();
  const [form, setForm] = useState<CreateGroundStop>(EMPTY);

  function set<K extends keyof CreateGroundStop>(key: K, value: string) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  function submit() {
    const airport = (form.airport ?? "").replace(/[^a-zA-Z0-9]/g, "");
    if (airport.length < 3 || airport.length > 4) {
      toast.warning("Enter a 3–4 character airport ICAO");
      return;
    }
    create.mutate(
      { airport, scope: form.scope, until: form.until },
      { onSuccess: () => setForm(EMPTY) },
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-end gap-3">
        <label className={LABEL}>
          Airport
          <Input
            className="w-24 font-mono uppercase"
            maxLength={4}
            placeholder="KDCA"
            value={form.airport}
            onChange={(e) => set("airport", e.target.value)}
          />
        </label>
        <label className={LABEL}>
          Scope (ARTCC/FIR)
          <Input
            className="w-48 font-mono uppercase"
            placeholder="ZTL ZJX"
            value={form.scope ?? ""}
            onChange={(e) => set("scope", e.target.value)}
          />
        </label>
        <label className={LABEL}>
          Until (Zxxxx)
          <Input
            className="w-24 font-mono"
            placeholder="0200z"
            value={form.until ?? ""}
            onChange={(e) => set("until", e.target.value)}
          />
        </label>
        <Button className="whitespace-nowrap" disabled={create.isPending} onClick={submit}>
          <Plus />
          Add ground stop
        </Button>
      </div>
      <p className="text-xs text-ink-3">
        Holds GROUND departures into the named airport that originate inside the scoped ARTCC/FIR(s).
        Leave SCOPE blank for a field-wide stop.
      </p>
    </div>
  );
}

function GroundStopActions({
  gs,
  canPublish,
  canDelete,
}: {
  gs: GroundStop;
  canPublish: boolean;
  canDelete: boolean;
}) {
  const publish = usePublishGroundStop();
  const cancel = useCancelGroundStop();
  const del = useDeleteGroundStop();

  return (
    <div className="flex justify-end gap-1">
      {canPublish && gs.status === "draft" && (
        <Button size="sm" variant="secondary" disabled={publish.isPending} onClick={() => publish.mutate(gs.id)}>
          Publish
        </Button>
      )}
      {canPublish && (gs.status === "draft" || gs.status === "published") && (
        <Button size="sm" variant="ghost" disabled={cancel.isPending} onClick={() => cancel.mutate(gs.id)}>
          Cancel
        </Button>
      )}
      {canDelete && (
        <ConfirmButton size="sm" onConfirm={() => del.mutate(gs.id)} warn={`Delete the ${gs.airport} ground stop?`}>
          Delete
        </ConfirmButton>
      )}
    </div>
  );
}

export function GroundStopsTab() {
  const { data: me } = useMe();
  const stops = useGroundStops();
  const canCreate = hasPermission(me, "tmu.groundstop.create");
  const canPublish = hasPermission(me, "tmu.groundstop.publish");
  const canDelete = hasPermission(me, "tmu.groundstop.delete");

  const columns = useMemo<DataColumn<GroundStop>[]>(
    () => [
      {
        accessorKey: "status",
        header: "Status",
        icon: CircleDot,
        cell: (c) => <StatusPill tone={toneOf("publish", c.getValue<string>())}>{c.getValue<string>()}</StatusPill>,
      },
      {
        accessorKey: "airport",
        header: "Airport",
        icon: Plane,
        mono: true,
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      {
        accessorKey: "scope",
        header: "Scope (ARTCC/FIR)",
        icon: Radar,
        mono: true,
        cell: (c) => c.getValue<string>() || <span className="font-sans text-ink-3">All departures</span>,
      },
      {
        accessorKey: "until",
        header: "Until (Zxxxx)",
        icon: Clock,
        mono: true,
        cell: (c) => (c.getValue<string>() ? `${c.getValue<string>()}z` : <span className="text-ink-3">UFN</span>),
      },
      {
        accessorKey: "updated_at",
        header: "Updated",
        mono: true,
        cell: (c) => (
          <span className="whitespace-nowrap text-ink-3">
            {formatZulu(c.getValue<string>())}
            {c.row.original.updated_by ? ` · ${c.row.original.updated_by}` : ""}
          </span>
        ),
      },
      {
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => <GroundStopActions gs={c.row.original} canPublish={canPublish} canDelete={canDelete} />,
      },
    ],
    [canPublish, canDelete],
  );

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm />}

      <DataTable
        label="Ground stops"
        columns={columns}
        data={stops.data ?? []}
        getRowId={(gs) => gs.id}
        rowCap={25}
        isLoading={!stops.data}
        isError={stops.isError}
        onRetry={() => stops.refetch()}
        empty="No active ground stops."
      />
    </div>
  );
}

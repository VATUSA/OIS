import {useState} from "react";
import {Badge, Button, Card, CardContent, CardHeader, CardTitle, Input,} from "@ois/ui";
import {Plus} from "lucide-react";

import {useFacilities} from "@/lib/admin";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type CreateTmi, type Tmi, useCancelTmi, useCreateTmi, useDeleteTmi, usePublishTmi, useTmis,} from "@/lib/tmu";

const KINDS = [
  "MIT",
  "MINIT",
  "Ground Stop",
  "Ground Delay",
  "Reroute",
  "Other",
];

function statusVariant(
  status: string,
): "secondary" | "success" | "destructive" | "outline" {
  if (status === "published") return "success";
  if (status === "cancelled") return "destructive";
  if (status === "expired") return "outline";
  return "secondary";
}

const EMPTY: CreateTmi = {
  kind: "MIT",
  element: "",
  restriction: "",
  reason: "",
  artcc_id: null,
};

function CreateForm() {
  const facilities = useFacilities();
  const create = useCreateTmi();
  const [form, setForm] = useState<CreateTmi>(EMPTY);

  const valid =
    !!form.kind && !!form.element?.trim() && !!form.restriction?.trim();

  const field = "h-9 rounded-md border border-input bg-background px-3 text-sm";

  return (
    <Card>
      <CardHeader>
        <CardTitle>New TMI</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            Kind
            <select
              className={field}
              value={form.kind}
              onChange={(e) => setForm({ ...form, kind: e.target.value })}
            >
              {KINDS.map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            Element
            <Input
              className="w-32"
              placeholder="ORD / OTT"
              value={form.element}
              onChange={(e) => setForm({ ...form, element: e.target.value })}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            Restriction
            <Input
              className="w-40"
              placeholder="20 MIT"
              value={form.restriction}
              onChange={(e) => setForm({ ...form, restriction: e.target.value })}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            ARTCC
            <select
              className={field}
              value={form.artcc_id ?? ""}
              onChange={(e) =>
                setForm({ ...form, artcc_id: e.target.value || null })
              }
            >
              <option value="">National</option>
              {facilities.data?.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.id}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-1 flex-col gap-1 text-xs font-medium text-muted-foreground">
            Reason
            <Input
              placeholder="Optional"
              value={form.reason ?? ""}
              onChange={(e) => setForm({ ...form, reason: e.target.value })}
            />
          </label>
          <Button
            disabled={!valid || create.isPending}
            onClick={() =>
              create.mutate(form, { onSuccess: () => setForm(EMPTY) })
            }
          >
            <Plus />
            Add draft
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function TmiRow({
  tmi,
  canPublish,
  canDelete,
}: {
  tmi: Tmi;
  canPublish: boolean;
  canDelete: boolean;
}) {
  const publish = usePublishTmi();
  const cancel = useCancelTmi();
  const del = useDeleteTmi();

  return (
    <tr className="border-t">
      <td className="py-2 pr-3">
        <Badge variant={statusVariant(tmi.status)}>{tmi.status}</Badge>
      </td>
      <td className="py-2 pr-3 font-medium">{tmi.kind}</td>
      <td className="py-2 pr-3 font-mono text-xs">{tmi.element}</td>
      <td className="py-2 pr-3">{tmi.restriction}</td>
      <td className="py-2 pr-3 text-muted-foreground">
        {tmi.artcc_id ?? "National"}
      </td>
      <td className="py-2 pr-3 text-muted-foreground">{tmi.author ?? "—"}</td>
      <td className="py-2 text-right">
        <div className="flex justify-end gap-1">
          {canPublish && tmi.status === "draft" && (
            <Button
              size="sm"
              variant="secondary"
              disabled={publish.isPending}
              onClick={() => publish.mutate(tmi.id)}
            >
              Publish
            </Button>
          )}
          {canPublish && (tmi.status === "draft" || tmi.status === "published") && (
            <Button
              size="sm"
              variant="ghost"
              disabled={cancel.isPending}
              onClick={() => cancel.mutate(tmi.id)}
            >
              Cancel
            </Button>
          )}
          {canDelete && (
            <Button
              size="sm"
              variant="ghost"
              className="text-destructive hover:text-destructive"
              disabled={del.isPending}
              onClick={() => del.mutate(tmi.id)}
            >
              Delete
            </Button>
          )}
        </div>
      </td>
    </tr>
  );
}

export function TmuPage() {
  const { data: me } = useMe();
  const tmis = useTmis();
  const canCreate = hasPermission(me, "tmu.tmi.create");
  const canPublish = hasPermission(me, "tmu.tmi.publish");
  const canDelete = hasPermission(me, "tmu.tmi.delete");

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          Traffic Management
        </h1>
        <p className="text-muted-foreground">
          Traffic Management Initiatives (TMIs). Draft, then publish.
        </p>
      </div>

      {canCreate && <CreateForm />}

      <Card>
        <CardContent className="pt-6">
          {tmis.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Couldn&apos;t load TMIs.
            </p>
          ) : !tmis.data ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Loading…
            </p>
          ) : tmis.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No TMIs yet.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Status</th>
                    <th className="pb-2 pr-3 font-medium">Kind</th>
                    <th className="pb-2 pr-3 font-medium">Element</th>
                    <th className="pb-2 pr-3 font-medium">Restriction</th>
                    <th className="pb-2 pr-3 font-medium">ARTCC</th>
                    <th className="pb-2 pr-3 font-medium">Author</th>
                    <th className="pb-2" />
                  </tr>
                </thead>
                <tbody>
                  {tmis.data.map((tmi) => (
                    <TmiRow
                      key={tmi.id}
                      tmi={tmi}
                      canPublish={canPublish}
                      canDelete={canDelete}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

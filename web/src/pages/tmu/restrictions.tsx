import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input, useToast} from "@ois/ui";
import {Plus} from "lucide-react";

import {hasPermission} from "@/lib/permissions";
import {formatZulu, parseZulu} from "@/lib/time";
import {useMe} from "@/lib/auth";
import {type Tmi, useCancelTmi, useCreateTmi, useDeleteTmi, usePublishTmi, useTmis,} from "@/lib/tmu";

function statusVariant(
  status: string,
): "secondary" | "success" | "destructive" | "outline" {
  if (status === "published") return "success";
  if (status === "cancelled") return "destructive";
  if (status === "expired") return "outline";
  return "secondary";
}

type FormState = {
  requesting: string;
  providing: string;
  restriction: string;
  start: string;
  stop: string;
};

const EMPTY: FormState = {
  requesting: "",
  providing: "",
  restriction: "",
  start: "",
  stop: "",
};

const COLS =
  "grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,3fr)_minmax(0,1.2fr)_minmax(0,1.2fr)_auto] items-end gap-3";

function Head({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
      {children}
    </span>
  );
}

function CreateForm() {
  const create = useCreateTmi();
  const toast = useToast();
  const [form, setForm] = useState<FormState>(EMPTY);

  function set<K extends keyof FormState>(key: K, value: string) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  function submit() {
    if (
      !form.requesting.trim() ||
      !form.providing.trim() ||
      !form.restriction.trim()
    ) {
      toast.warning("Requesting, providing, and restriction are required");
      return;
    }
    const start = form.start.trim() ? parseZulu(form.start) : null;
    const stop = form.stop.trim() ? parseZulu(form.stop) : null;
    if (form.start.trim() && !start) {
      toast.warning("Start time must be DD/HHMMz (e.g. 12/1430z)");
      return;
    }
    if (form.stop.trim() && !stop) {
      toast.warning("Stop time must be DD/HHMMz (e.g. 12/1830z)");
      return;
    }
    create.mutate(
      {
        requesting: form.requesting,
        providing: form.providing,
        restriction: form.restriction,
        start_time: start,
        stop_time: stop,
      },
      { onSuccess: () => setForm(EMPTY) },
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="overflow-x-auto">
          <div className={`${COLS} min-w-[860px]`}>
            <Head>Requesting</Head>
            <Head>Providing</Head>
            <Head>Restriction</Head>
            <Head>Start time</Head>
            <Head>Stop time</Head>
            <span />

            <Input
              placeholder="ARTCC/TRACON"
              value={form.requesting}
              onChange={(e) => set("requesting", e.target.value)}
            />
            <Input
              placeholder="ARTCC/TRACON"
              value={form.providing}
              onChange={(e) => set("providing", e.target.value)}
            />
            <Input
              placeholder="e.g. 20 MIT jets / 250kt"
              value={form.restriction}
              onChange={(e) => set("restriction", e.target.value)}
            />
            <Input
              placeholder="DD/HHMMz"
              value={form.start}
              onChange={(e) => set("start", e.target.value)}
            />
            <Input
              placeholder="DD/HHMMz"
              value={form.stop}
              onChange={(e) => set("stop", e.target.value)}
            />
            <Button
              className="whitespace-nowrap"
              disabled={create.isPending}
              onClick={submit}
            >
              <Plus />
              Add restriction
            </Button>
          </div>
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
      <td className="py-2 pr-3 font-mono text-xs">{tmi.requesting}</td>
      <td className="py-2 pr-3 font-mono text-xs">{tmi.providing}</td>
      <td className="py-2 pr-3">{tmi.restriction}</td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(tmi.start_time)}
      </td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(tmi.stop_time)}
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
          {canPublish &&
            (tmi.status === "draft" || tmi.status === "published") && (
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
            <ConfirmButton
              size="sm"
              onConfirm={() => del.mutate(tmi.id)}
              warn="Delete this restriction?"
            >
              Delete
            </ConfirmButton>
          )}
        </div>
      </td>
    </tr>
  );
}

export function RestrictionsTab() {
  const { data: me } = useMe();
  const tmis = useTmis();
  const canCreate = hasPermission(me, "tmu.tmi.create");
  const canPublish = hasPermission(me, "tmu.tmi.publish");
  const canDelete = hasPermission(me, "tmu.tmi.delete");

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm />}

      <Card>
        <CardContent className="pt-6">
          {tmis.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Couldn&apos;t load restrictions.
            </p>
          ) : !tmis.data ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Loading…
            </p>
          ) : tmis.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No restrictions yet.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Status</th>
                    <th className="pb-2 pr-3 font-medium">Requesting</th>
                    <th className="pb-2 pr-3 font-medium">Providing</th>
                    <th className="pb-2 pr-3 font-medium">Restriction</th>
                    <th className="pb-2 pr-3 font-medium">Start</th>
                    <th className="pb-2 pr-3 font-medium">Stop</th>
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

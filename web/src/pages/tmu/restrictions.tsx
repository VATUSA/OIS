import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input, useToast} from "@ois/ui";
import {Plus} from "lucide-react";

import {hasPermission} from "@/lib/permissions";
import {formatZulu, parseZulu} from "@/lib/time";
import {useMe} from "@/lib/auth";
import {
  type Tmi,
  type TmiFilters,
  useCancelTmi,
  useCreateTmi,
  useDeleteTmi,
  usePublishTmi,
  useTmis,
} from "@/lib/tmu";
import {NtmlEditor} from "@/components/ntml-editor";
import {EMPTY_NTML, KINDS, type Ntml} from "@/lib/ntml";

const SELECT_CLASS =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";
const STATUSES = ["draft", "published", "expired", "cancelled"] as const;

function statusVariant(
  status: string,
): "secondary" | "success" | "destructive" | "outline" {
  if (status === "published") return "success";
  if (status === "cancelled") return "destructive";
  if (status === "expired") return "outline";
  return "secondary";
}

type FormState = { requesting: string; providing: string; restriction: string; start: string; stop: string };
const EMPTY: FormState = { requesting: "", providing: "", restriction: "", start: "", stop: "" };

function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

function CreateForm() {
  const create = useCreateTmi();
  const toast = useToast();
  const [mode, setMode] = useState<"structured" | "raw">("structured");
  const [form, setForm] = useState<FormState>(EMPTY);
  const [ntml, setNtml] = useState<Ntml>(EMPTY_NTML);

  const set = <K extends keyof FormState>(key: K, value: string) =>
    setForm((f) => ({ ...f, [key]: value }));

  function submit() {
    if (!form.requesting.trim() || !form.providing.trim()) {
      toast.warning("Requesting and providing are required");
      return;
    }
    if (mode === "raw" && !form.restriction.trim()) {
      toast.warning("A restriction is required");
      return;
    }
    if (mode === "structured" && (!ntml.element.trim() || !ntml.kind.trim())) {
      toast.warning("An element and a restriction type are required");
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
        restriction: mode === "raw" ? form.restriction : "",
        structured: mode === "structured" ? ntml : undefined,
        start_time: start,
        stop_time: stop,
      },
      {
        onSuccess: () => {
          setForm(EMPTY);
          setNtml(EMPTY_NTML);
        },
      },
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex overflow-hidden rounded-md border">
            {(["structured", "raw"] as const).map((m) => (
              <button
                key={m}
                type="button"
                onClick={() => setMode(m)}
                className={
                  "px-3 py-1.5 text-sm font-medium transition-colors " +
                  (mode === m ? "bg-primary text-primary-foreground" : "hover:bg-accent/40")
                }
              >
                {m === "structured" ? "Structured" : "Raw"}
              </button>
            ))}
          </div>
          <Labeled label="Requesting">
            <Input className="w-32" placeholder="ARTCC/TRACON" value={form.requesting} onChange={(e) => set("requesting", e.target.value)} />
          </Labeled>
          <Labeled label="Providing">
            <Input className="w-32" placeholder="ARTCC/TRACON" value={form.providing} onChange={(e) => set("providing", e.target.value)} />
          </Labeled>
          <Labeled label="Start time">
            <Input className="w-28" placeholder="DD/HHMMz" value={form.start} onChange={(e) => set("start", e.target.value)} />
          </Labeled>
          <Labeled label="Stop time">
            <Input className="w-28" placeholder="DD/HHMMz" value={form.stop} onChange={(e) => set("stop", e.target.value)} />
          </Labeled>
        </div>

        {mode === "raw" ? (
          <Labeled label="Restriction (raw NTML line)">
            <Input
              placeholder="e.g. JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL"
              value={form.restriction}
              onChange={(e) => set("restriction", e.target.value)}
            />
          </Labeled>
        ) : (
          <NtmlEditor value={ntml} onChange={setNtml} />
        )}

        <div className="flex justify-end">
          <Button disabled={create.isPending} onClick={submit}>
            <Plus />
            Add restriction
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
      <td className="py-2 pr-3 font-mono text-xs">{tmi.requesting}</td>
      <td className="py-2 pr-3 font-mono text-xs">{tmi.providing}</td>
      <td className="py-2 pr-3">
        <div className="font-mono text-xs">{tmi.restriction}</div>
        {tmi.decoded && <div className="text-xs text-muted-foreground">{tmi.decoded}</div>}
      </td>
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

/** Draft (in `YYYY-MM-DDTHH:mm` for the datetime-local inputs), converted to RFC 3339 on apply. */
type Draft = { status: string; type: string; facility: string; from: string; to: string };
const EMPTY_DRAFT: Draft = { status: "", type: "", facility: "", from: "", to: "" };

function toFilters(d: Draft): TmiFilters {
  return {
    status: d.status || undefined,
    type: d.type || undefined,
    facility: d.facility.trim() || undefined,
    from: d.from ? `${d.from}:00Z` : undefined,
    to: d.to ? `${d.to}:00Z` : undefined,
  };
}

function FilterBar({ onChange }: { onChange: (f: TmiFilters) => void }) {
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const set = <K extends keyof Draft>(key: K, value: string) =>
    setDraft((d) => ({ ...d, [key]: value }));
  const active = Object.values(draft).some(Boolean);

  return (
    <form
      className="flex flex-wrap items-end gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        onChange(toFilters(draft));
      }}
    >
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Status
        <select className={SELECT_CLASS} value={draft.status} onChange={(e) => set("status", e.target.value)}>
          <option value="">Any</option>
          {STATUSES.map((s) => (
            <option key={s} value={s}>{s}</option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Type
        <select className={SELECT_CLASS} value={draft.type} onChange={(e) => set("type", e.target.value)}>
          <option value="">Any</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>{k}</option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Facility
        <Input className="w-32" placeholder="ARTCC/TRACON" value={draft.facility} onChange={(e) => set("facility", e.target.value)} />
      </label>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Active from
        <Input type="datetime-local" value={draft.from} onChange={(e) => set("from", e.target.value)} />
      </label>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Active to
        <Input type="datetime-local" value={draft.to} onChange={(e) => set("to", e.target.value)} />
      </label>
      <Button type="submit" size="sm">
        Filter
      </Button>
      {active && (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={() => {
            setDraft(EMPTY_DRAFT);
            onChange({});
          }}
        >
          Clear
        </Button>
      )}
    </form>
  );
}

export function RestrictionsTab() {
  const { data: me } = useMe();
  const [filters, setFilters] = useState<TmiFilters>({});
  const tmis = useTmis(filters);
  const canCreate = hasPermission(me, "tmu.tmi.create");
  const canPublish = hasPermission(me, "tmu.tmi.publish");
  const canDelete = hasPermission(me, "tmu.tmi.delete");
  const filtered = Object.values(filters).some(Boolean);

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm />}

      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <FilterBar onChange={setFilters} />
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
              {filtered ? "No restrictions match these filters." : "No restrictions yet."}
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

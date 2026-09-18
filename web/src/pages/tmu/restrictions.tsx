import {useMemo, useState} from "react";
import {useSearch} from "@tanstack/react-router";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  FilterBar,
  Input,
  SegmentedControl,
  Select,
  StatusPill,
  useToast,
} from "@ois/ui";
import {CircleDot, Clock, Plus, ShieldAlert, User} from "lucide-react";

import {ZuluDateTime} from "@/components/zulu-datetime";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
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

const STATUSES = ["draft", "published", "expired", "cancelled"] as const;

type FormState = { requesting: string; providing: string; restriction: string; start: string; stop: string };
const EMPTY: FormState = { requesting: "", providing: "", restriction: "", start: "", stop: "" };

function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-ink-2">
      {label}
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
    <Card className="flex flex-col gap-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-xl font-bold">New restriction</h2>
        <SegmentedControl
          aria-label="Entry mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "structured", label: "Structured" },
            { value: "raw", label: "Raw" },
          ]}
        />
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <Labeled label="Requesting">
          <Input className="w-32 font-mono uppercase" placeholder="ARTCC/TRACON" value={form.requesting} onChange={(e) => set("requesting", e.target.value)} />
        </Labeled>
        <Labeled label="Providing">
          <Input className="w-32 font-mono uppercase" placeholder="ARTCC/TRACON" value={form.providing} onChange={(e) => set("providing", e.target.value)} />
        </Labeled>
        <Labeled label="Start time">
          <Input className="w-28 font-mono" placeholder="DD/HHMMz" value={form.start} onChange={(e) => set("start", e.target.value)} />
        </Labeled>
        <Labeled label="Stop time">
          <Input className="w-28 font-mono" placeholder="DD/HHMMz" value={form.stop} onChange={(e) => set("stop", e.target.value)} />
        </Labeled>
      </div>

      {mode === "raw" ? (
        <Labeled label="Restriction (raw NTML line)">
          <Input
            className="font-mono"
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
    </Card>
  );
}

function TmiActions({
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
    <div className="flex justify-end gap-1">
      {canPublish && tmi.status === "draft" && (
        <Button size="sm" variant="secondary" disabled={publish.isPending} onClick={() => publish.mutate(tmi.id)}>
          Publish
        </Button>
      )}
      {canPublish && (tmi.status === "draft" || tmi.status === "published") && (
        <Button size="sm" variant="ghost" disabled={cancel.isPending} onClick={() => cancel.mutate(tmi.id)}>
          Cancel
        </Button>
      )}
      {canDelete && (
        <ConfirmButton size="sm" onConfirm={() => del.mutate(tmi.id)} warn="Delete this restriction?">
          Delete
        </ConfirmButton>
      )}
    </div>
  );
}

/** Draft filters (window bounds as unix seconds, Zulu), converted to RFC 3339 on apply. */
type Draft = { status: string; type: string; facility: string; from: number | null; to: number | null };
const EMPTY_DRAFT: Draft = { status: "", type: "", facility: "", from: null, to: null };
const iso = (unixS: number | null) => (unixS != null ? new Date(unixS * 1000).toISOString() : undefined);

function toFilters(d: Draft): TmiFilters {
  return {
    status: d.status || undefined,
    type: d.type || undefined,
    facility: d.facility.trim() || undefined,
    from: iso(d.from),
    to: iso(d.to),
  };
}

/** Server-side filters — applied on submit, since each change is a new query. */
function RestrictionFilters({
  initialFacility = "",
  onChange,
}: {
  initialFacility?: string;
  onChange: (f: TmiFilters) => void;
}) {
  const [draft, setDraft] = useState<Draft>({ ...EMPTY_DRAFT, facility: initialFacility });
  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((d) => ({ ...d, [key]: value }));
  const active = Object.values(draft).some((v) => v != null && v !== "");

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onChange(toFilters(draft));
      }}
    >
      <FilterBar>
        <Select size="sm" aria-label="Status" value={draft.status} onChange={(e) => set("status", e.target.value)}>
          <option value="">Any status</option>
          {STATUSES.map((s) => (
            <option key={s} value={s}>{s}</option>
          ))}
        </Select>
        <Select size="sm" aria-label="Type" value={draft.type} onChange={(e) => set("type", e.target.value)}>
          <option value="">Any type</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>{k}</option>
          ))}
        </Select>
        <Input
          aria-label="Facility"
          className="h-8 w-36 font-mono uppercase"
          placeholder="ARTCC/TRACON"
          value={draft.facility}
          onChange={(e) => set("facility", e.target.value)}
        />
        <div className="flex items-center gap-1.5 text-xs text-ink-3">
          Active from
          <ZuluDateTime label="Active from" value={draft.from} onChange={(v) => set("from", v)} onClear={() => set("from", null)} />
        </div>
        <div className="flex items-center gap-1.5 text-xs text-ink-3">
          to
          <ZuluDateTime label="Active to" value={draft.to} onChange={(v) => set("to", v)} onClear={() => set("to", null)} />
        </div>
        <Button type="submit" size="sm" variant="outline">
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
      </FilterBar>
    </form>
  );
}

export function RestrictionsTab() {
  const { data: me } = useMe();
  // `?facility=` (the ⌘K search jumping to a TMI, deep links) opens the list already filtered, and
  // seeds the bar so the applied filter is visible and clearable.
  const initialFacility = useSearch({ from: "/ops/tmu" }).facility ?? "";
  const [filters, setFilters] = useState<TmiFilters>(initialFacility ? { facility: initialFacility } : {});
  const tmis = useTmis(filters);
  const canCreate = hasPermission(me, "tmu.tmi.create");
  const canPublish = hasPermission(me, "tmu.tmi.publish");
  const canDelete = hasPermission(me, "tmu.tmi.delete");
  const filtered = Object.values(filters).some(Boolean);

  const columns = useMemo<DataColumn<Tmi>[]>(
    () => [
      {
        accessorKey: "status",
        header: "Status",
        icon: CircleDot,
        cell: (c) => <StatusPill tone={toneOf("publish", c.getValue<string>())}>{c.getValue<string>()}</StatusPill>,
      },
      { accessorKey: "requesting", header: "Requesting", mono: true },
      { accessorKey: "providing", header: "Providing", mono: true },
      {
        accessorKey: "restriction",
        header: "Restriction",
        icon: ShieldAlert,
        cell: (c) => (
          <div className="min-w-64">
            <div className="font-mono text-[13px]">{c.getValue<string>()}</div>
            {c.row.original.decoded && <div className="text-xs text-ink-3">{c.row.original.decoded}</div>}
          </div>
        ),
      },
      {
        accessorKey: "start_time",
        header: "Start",
        icon: Clock,
        mono: true,
        cell: (c) => <span className="whitespace-nowrap text-ink-2">{formatZulu(c.getValue<string>())}</span>,
      },
      {
        accessorKey: "stop_time",
        header: "Stop",
        mono: true,
        cell: (c) => <span className="whitespace-nowrap text-ink-2">{formatZulu(c.getValue<string>())}</span>,
      },
      {
        accessorKey: "author",
        header: "Author",
        icon: User,
        cell: (c) => <span className="text-ink-2">{c.getValue<string>() ?? "—"}</span>,
      },
      {
        id: "actions",
        header: "",
        enableSorting: false,
        align: "right",
        cell: (c) => <TmiActions tmi={c.row.original} canPublish={canPublish} canDelete={canDelete} />,
      },
    ],
    [canPublish, canDelete],
  );

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-3">
        <RestrictionFilters initialFacility={initialFacility} onChange={setFilters} />
        <DataTable
          label="Restrictions"
          columns={columns}
          data={tmis.data ?? []}
          getRowId={(t) => t.id}
          rowCap={25}
          isLoading={!tmis.data}
          isError={tmis.isError}
          onRetry={() => tmis.refetch()}
          empty={filtered ? "No restrictions match these filters." : "No restrictions yet."}
        />
      </div>

      {canCreate && <CreateForm />}
    </div>
  );
}

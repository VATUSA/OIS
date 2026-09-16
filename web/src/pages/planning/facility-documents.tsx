import {useMemo, useState} from "react";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  EmptyState,
  FilterBar,
  Input,
  Select,
} from "@ois/ui";
import {Building2, FileText, Link2, Lock, Plus, X} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {
  type FacilityDocument,
  type UpsertFacilityDocument,
  useCreateFacilityDocument,
  useDeleteFacilityDocument,
  useFacilityDocuments,
  useUpdateFacilityDocument,
} from "@/lib/facility-documents";
import {hasPermission} from "@/lib/permissions";

const SUBTITLE =
  "Reference documents configured per facility, sent to controllers who cover an ACE request for that facility.";

const BLANK: UpsertFacilityDocument = { title: "", url: "" };

function DocumentForm({
  initial,
  editingId,
  onCancel,
  onSave,
  pending,
}: {
  initial: UpsertFacilityDocument;
  editingId: string | null;
  onCancel: () => void;
  onSave: (body: UpsertFacilityDocument) => void;
  pending: boolean;
}) {
  const [f, setF] = useState<UpsertFacilityDocument>(initial);
  const set = (patch: Partial<UpsertFacilityDocument>) => setF((p) => ({ ...p, ...patch }));

  return (
    <Card className="flex flex-col gap-3 p-4">
      <h2 className="text-xl font-bold">{editingId ? "Edit document" : "New document"}</h2>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1 text-xs">
          <span className="font-semibold text-ink-2">Title</span>
          <Input value={f.title} onChange={(e) => set({ title: e.target.value })} placeholder="Facility SOP" />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="font-semibold text-ink-2">URL</span>
          <Input value={f.url} onChange={(e) => set({ url: e.target.value })} placeholder="https://…" />
        </label>
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" disabled={!f.title.trim() || !f.url.trim() || pending} onClick={() => onSave(f)}>
          {editingId ? "Save" : "Add document"}
        </Button>
      </div>
    </Card>
  );
}

function FacilityDocuments({ facilityId, controls }: { facilityId: string; controls: React.ReactNode }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "facilities.docs.update");
  const docs = useFacilityDocuments(facilityId);
  const create = useCreateFacilityDocument(facilityId);
  const update = useUpdateFacilityDocument(facilityId);
  const { mutate: del } = useDeleteFacilityDocument(facilityId);
  const [form, setForm] = useState<null | { id: string | null; initial: UpsertFacilityDocument }>(null);

  const rows = docs.data ?? [];
  const editable = rows[0]?.editable ?? canEdit;

  const save = (body: UpsertFacilityDocument) => {
    const done = () => setForm(null);
    if (form?.id) update.mutate({ id: form.id, body }, { onSuccess: done });
    else create.mutate(body, { onSuccess: done });
  };

  const columns = useMemo<DataColumn<FacilityDocument>[]>(
    () => [
      {
        accessorKey: "title",
        header: "Title",
        icon: FileText,
        cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
      },
      {
        accessorKey: "url",
        header: "URL",
        icon: Link2,
        enableSorting: false,
        cell: (c) => (
          <a
            href={c.getValue<string>()}
            target="_blank"
            rel="noreferrer"
            className="block max-w-[32rem] truncate text-xs text-brand-ink hover:underline"
          >
            {c.getValue<string>()}
          </a>
        ),
      },
      ...(editable
        ? [
            {
              id: "actions",
              header: () => <span className="sr-only">Actions</span>,
              enableSorting: false,
              align: "right",
              cell: (c) => {
                const d = c.row.original;
                return (
                  <div className="flex items-center justify-end gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2"
                      onClick={() => setForm({ id: d.id, initial: { title: d.title, url: d.url } })}
                    >
                      Edit
                    </Button>
                    <ConfirmButton
                      size="icon"
                      variant="ghost"
                      className="size-7 text-ink-3 hover:text-danger"
                      aria-label={`Delete ${d.title}`}
                      warn={`Delete "${d.title}"?`}
                      onConfirm={() => del(d.id)}
                    >
                      <X className="size-4" />
                    </ConfirmButton>
                  </div>
                );
              },
            } satisfies DataColumn<FacilityDocument>,
          ]
        : []),
    ],
    [editable, del],
  );

  return (
    <>
      <FilterBar>
        {controls}
        {editable && !form && (
          <Button size="sm" className="ml-auto" onClick={() => setForm({ id: null, initial: BLANK })}>
            <Plus className="size-3.5" />
            Add document
          </Button>
        )}
      </FilterBar>

      {form && (
        <DocumentForm
          initial={form.initial}
          editingId={form.id}
          onCancel={() => setForm(null)}
          onSave={save}
          pending={create.isPending || update.isPending}
        />
      )}

      <DataTable
        label={`${facilityId} documents`}
        columns={columns}
        data={rows}
        getRowId={(d) => d.id}
        rowCap={25}
        isLoading={docs.isLoading}
        isError={docs.isError}
        onRetry={() => docs.refetch()}
        empty={`No documents for ${facilityId} yet${editable ? " — add one above." : "."}`}
      />
    </>
  );
}

export function FacilityDocumentsPage() {
  const { data: me } = useMe();
  // Must match the backend gate (`facilities.docs.read`) — `events.plan.read` doesn't authorize
  // this endpoint, it would just make the page appear before every fetch 403s.
  const canRead = hasPermission(me, "facilities.docs.read");
  const facilities = useFacilities();
  const [facilityId, setFacilityId] = useState("");

  usePageHeader({ subtitle: SUBTITLE });

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have access to facility documents yet.</EmptyState>;
  }

  const artccs = (facilities.data ?? [])
    .filter((f) => f.active)
    .map((f) => f.id)
    .sort();

  const picker = (
    <Select aria-label="Facility" size="sm" value={facilityId} onChange={(e) => setFacilityId(e.target.value)}>
      <option value="">Select a facility…</option>
      {artccs.map((id) => (
        <option key={id} value={id}>
          {id}
        </option>
      ))}
    </Select>
  );

  return (
    <div className="flex flex-col gap-4">
      {facilityId ? (
        <FacilityDocuments key={facilityId} facilityId={facilityId} controls={picker} />
      ) : (
        <>
          <FilterBar>{picker}</FilterBar>
          <EmptyState icon={Building2} className="rounded-md border border-line">
            Pick a facility to see its documents.
          </EmptyState>
        </>
      )}
    </div>
  );
}

import {useState} from "react";
import {Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {FileText, Plus, X} from "lucide-react";

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

const SELECT_CLASS =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

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
    <div className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Title</span>
          <Input
            value={f.title}
            onChange={(e) => set({ title: e.target.value })}
            placeholder="Facility SOP"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">URL</span>
          <Input
            value={f.url}
            onChange={(e) => set({ url: e.target.value })}
            placeholder="https://…"
          />
        </label>
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          size="sm"
          disabled={!f.title.trim() || !f.url.trim() || pending}
          onClick={() => onSave(f)}
        >
          {editingId ? "Save" : "Add document"}
        </Button>
      </div>
    </div>
  );
}

function FacilityDocuments({ facilityId }: { facilityId: string }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "facilities.docs.update");
  const docs = useFacilityDocuments(facilityId);
  const create = useCreateFacilityDocument(facilityId);
  const update = useUpdateFacilityDocument(facilityId);
  const del = useDeleteFacilityDocument(facilityId);
  const [form, setForm] = useState<null | { id: string | null; initial: UpsertFacilityDocument }>(
    null,
  );

  const rows = docs.data ?? [];
  const editable = rows[0]?.editable ?? canEdit;

  const startEdit = (d: FacilityDocument) =>
    setForm({ id: d.id, initial: { title: d.title, url: d.url } });

  const save = (body: UpsertFacilityDocument) => {
    const done = () => setForm(null);
    if (form?.id) update.mutate({ id: form.id, body }, { onSuccess: done });
    else create.mutate(body, { onSuccess: done });
  };

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
              <FileText className="size-4" />
            </span>
            <div className="flex flex-col">
              <span className="font-semibold">{facilityId} documents</span>
              <span className="text-xs text-muted-foreground">
                Reference documents (SOPs, LOAs, etc.) shown to controllers who cover this facility.
              </span>
            </div>
          </div>
          {editable && !form && (
            <Button size="sm" onClick={() => setForm({ id: null, initial: BLANK })}>
              <Plus className="size-3.5" />
              Add document
            </Button>
          )}
        </div>

        {form && (
          <DocumentForm
            initial={form.initial}
            editingId={form.id}
            onCancel={() => setForm(null)}
            onSave={save}
            pending={create.isPending || update.isPending}
          />
        )}

        {!docs.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No documents for {facilityId} yet{editable ? " — add one above." : "."}
          </p>
        ) : (
          <div className="flex flex-col divide-y">
            {rows.map((d) => (
              <div key={d.id} className="flex items-center justify-between gap-3 py-2">
                <div className="flex min-w-0 flex-col">
                  <span className="font-medium">{d.title}</span>
                  <a
                    href={d.url}
                    target="_blank"
                    rel="noreferrer"
                    className="truncate text-xs text-muted-foreground hover:underline"
                  >
                    {d.url}
                  </a>
                </div>
                {editable && (
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2"
                      onClick={() => startEdit(d)}
                    >
                      Edit
                    </Button>
                    <ConfirmButton
                      size="icon"
                      variant="ghost"
                      className="size-7 text-muted-foreground hover:text-destructive"
                      warn={`Delete "${d.title}"?`}
                      onConfirm={() => del.mutate(d.id)}
                    >
                      <X className="size-4" />
                    </ConfirmButton>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function FacilityDocumentsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "events.plan.read");
  const facilities = useFacilities();
  const [facilityId, setFacilityId] = useState("");

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have event planning access yet.
        </CardContent>
      </Card>
    );
  }

  const artccs = (facilities.data ?? [])
    .filter((f) => f.active)
    .map((f) => f.id)
    .sort();

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Facility documents</h1>
        <p className="text-muted-foreground">
          Reference documents configured per facility, sent to controllers who cover an ACE request
          for that facility.
        </p>
      </div>

      <label className="flex w-fit flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Facility</span>
        <select
          className={SELECT_CLASS}
          value={facilityId}
          onChange={(e) => setFacilityId(e.target.value)}
        >
          <option value="">Select a facility…</option>
          {artccs.map((id) => (
            <option key={id} value={id}>
              {id}
            </option>
          ))}
        </select>
      </label>

      {facilityId && <FacilityDocuments key={facilityId} facilityId={facilityId} />}
    </div>
  );
}

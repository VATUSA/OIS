import {useMemo, useState} from "react";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  Input,
  SegmentedControl,
  StatusPill,
} from "@ois/ui";
import {CircleDot, FileText, Hash, Plus} from "lucide-react";

import {AdvisoryEditor} from "@/components/advisory-editor";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {toneOf} from "@/lib/status";
import {
  ADVISORY_KIND_REROUTE,
  type Advisory,
  EMPTY_REROUTE,
  type Reroute,
  useAdvisories,
  useCancelAdvisory,
  useCreateAdvisory,
  useDeleteAdvisory,
  usePublishAdvisory,
} from "@/lib/advisories";

type Mode = "structured" | "raw";

/**
 * Authoring surface for ADVZY advisories (VATUSA/OIS#460).
 *
 * A tab rather than its own route, because that is how every other TMU surface works — and the tab is
 * registered behind `tmu.adv.read` in `tmu.tsx`, so a non-holder never sees it. `/advisories` remains
 * the pilot-facing board; #456 settled that.
 */
export function AdvisoriesTab() {
  const { data: me } = useMe();
  const canCreate = hasPermission(me, "tmu.adv.create");
  const canPublish = hasPermission(me, "tmu.adv.publish");
  const canUpdate = hasPermission(me, "tmu.adv.update");

  const advisories = useAdvisories();
  const create = useCreateAdvisory();
  const publish = usePublishAdvisory();
  const cancel = useCancelAdvisory();
  const del = useDeleteAdvisory();

  const [mode, setMode] = useState<Mode>("structured");
  const [facility, setFacility] = useState("");
  const [structured, setStructured] = useState<Reroute>(EMPTY_REROUTE);
  const [raw, setRaw] = useState("");
  // The id of the draft just saved, so the preview shows *that* document rather than a guess.
  const [previewId, setPreviewId] = useState<string | null>(null);

  const preview = (advisories.data ?? []).find((a) => a.id === previewId) ?? null;

  const columns = useMemo<DataColumn<Advisory>[]>(
    () => [
      {
        accessorKey: "status",
        header: "Status",
        icon: CircleDot,
        cell: (c) => (
          <StatusPill tone={toneOf("publish", c.getValue<string>())}>{c.getValue<string>()}</StatusPill>
        ),
      },
      { accessorKey: "number", header: "No.", icon: Hash, mono: true },
      { accessorKey: "facility", header: "Facility", mono: true },
      { accessorKey: "kind", header: "Kind" },
      { accessorKey: "issued_day", header: "Issued", mono: true },
      {
        id: "actions",
        header: "",
        cell: (c) => {
          const a = c.row.original;
          return (
            <div className="flex items-center gap-2">
              <Button size="sm" variant="outline" onClick={() => setPreviewId(a.id)}>
                Preview
              </Button>
              {canPublish && a.status === "draft" && (
                <ConfirmButton size="sm" onConfirm={() => publish.mutate(a.id)} warn="Publish this advisory?">
                  Publish
                </ConfirmButton>
              )}
              {canPublish && a.status === "published" && (
                <ConfirmButton size="sm" onConfirm={() => cancel.mutate(a.id)} warn="Cancel this advisory?">
                  Cancel
                </ConfirmButton>
              )}
              {canUpdate && a.status === "draft" && (
                <ConfirmButton size="sm" onConfirm={() => del.mutate(a.id)} warn="Discard this draft?">
                  Discard
                </ConfirmButton>
              )}
            </div>
          );
        },
      },
    ],
    [canPublish, canUpdate, publish, cancel, del],
  );

  function save() {
    // `body` is required by the API even for a structured advisory, where the backend derives and
    // overwrites it from `structured` — so send the raw text in raw mode and a placeholder otherwise.
    create.mutate(
      mode === "raw"
        ? { facility, kind: ADVISORY_KIND_REROUTE, body: raw }
        : { facility, kind: ADVISORY_KIND_REROUTE, body: "(rendered on save)", structured },
      { onSuccess: (a) => setPreviewId(a.id) },
    );
  }

  const ready = facility.trim() !== "" && (mode === "raw" ? raw.trim() !== "" : structured.name.trim() !== "");

  return (
    <div className="flex flex-col gap-4">
      {canCreate && (
        <Card className="flex flex-col gap-3 p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h2 className="text-xl font-bold">New advisory</h2>
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

          <label className="flex flex-col gap-1 text-xs font-semibold text-ink-2">
            Issuing facility
            <Input
              placeholder="DCC"
              value={facility}
              onChange={(e) => setFacility(e.target.value)}
            />
          </label>

          {mode === "structured" ? (
            <AdvisoryEditor value={structured} onChange={setStructured} />
          ) : (
            <label className="flex flex-col gap-1 text-xs font-semibold text-ink-2">
              Document text
              <textarea
                aria-label="Document text"
                className="min-h-40 rounded-md border border-line bg-panel-2 p-2 font-mono text-xs text-ink"
                value={raw}
                onChange={(e) => setRaw(e.target.value)}
              />
            </label>
          )}

          <div className="flex items-center gap-2">
            <Button disabled={!ready || create.isPending} onClick={save}>
              <Plus className="size-4" /> Save draft
            </Button>
            <span className="text-xs text-ink-3">
              The number is taken at draft, so it is the number you will issue under.
            </span>
          </div>
        </Card>
      )}

      {preview && (
        <Card className="flex flex-col gap-2 p-4">
          <div className="flex items-center gap-2">
            <FileText className="size-4 text-ink-2" />
            <h2 className="text-lg font-bold">
              Advisory {preview.number} · {preview.facility}
            </h2>
            <StatusPill tone={toneOf("publish", preview.status)}>{preview.status}</StatusPill>
          </div>
          {/* The document exactly as it will post: `body` as the backend rendered it, never a
              locally rendered copy. `advisory_body` derives it from `structured`, so the fields and
              the document cannot drift. */}
          <pre aria-label="Rendered advisory" className="overflow-x-auto whitespace-pre rounded-md bg-panel-2 p-3 font-mono text-xs text-ink">
            {preview.body}
          </pre>
        </Card>
      )}

      <DataTable
        label="Advisories"
        columns={columns}
        data={advisories.data ?? []}
        getRowId={(a) => a.id}
        rowCap={25}
        isLoading={!advisories.data}
      />
    </div>
  );
}

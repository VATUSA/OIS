import {useState} from "react";
import {Button, ConfirmButton, Modal, QueryState, useToast} from "@ois/ui";

import {DiagnosticsTable} from "@/components/admin/diagnostics-table";
import {usePageHeader} from "@/components/shell/page-meta";
import {
  downloadDiagnosticsLogs,
  useDeleteDiagnosticsReport,
  useDiagnosticsReport,
  useDiagnosticsReports,
} from "@/lib/admin";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {formatZuluFull} from "@/lib/time";

const PAGE_SIZE = 50;
const SUBTITLE = "Reports sent from the desktop app with “Send diagnostics”. Kept for 30 days.";

/** Desktop diagnostics reports (#629). The nav entry's `diagnostics.reports.read` gates the page. */
export function AdminDiagnostics() {
  const [page, setPage] = useState(1);
  const [openId, setOpenId] = useState<string | null>(null);
  const reports = useDiagnosticsReports(page, PAGE_SIZE);

  usePageHeader({ subtitle: SUBTITLE, count: reports.data?.total ?? null });

  return (
    <div className="flex flex-col gap-4">
      <DiagnosticsTable
        items={reports.data?.items ?? []}
        isLoading={reports.isLoading}
        isError={reports.isError}
        rowCap={PAGE_SIZE}
        onOpen={setOpenId}
        serverPagination={
          reports.data
            ? {
                page: reports.data.page,
                pageSize: reports.data.page_size,
                total: reports.data.total,
                onPageChange: setPage,
              }
            : undefined
        }
      />
      <ReportModal id={openId} onClose={() => setOpenId(null)} />
    </div>
  );
}

function ReportModal({ id, onClose }: { id: string | null; onClose: () => void }) {
  const toast = useToast();
  const { data: me } = useMe();
  const report = useDiagnosticsReport(id);
  const remove = useDeleteDiagnosticsReport();
  const canDelete = hasPermission(me ?? null, "diagnostics.reports.delete");
  const r = report.data;

  const download = async () => {
    try {
      await downloadDiagnosticsLogs(id!);
    } catch {
      toast.error("Couldn't download the logs");
    }
  };

  return (
    <Modal
      open={id != null}
      onClose={onClose}
      size="lg"
      title="Diagnostics report"
      description={r ? `${r.user_display_name} (${r.user_cid}) · ${formatZuluFull(r.created_at)}` : undefined}
      footer={
        r && (
          <>
            {canDelete && (
              <ConfirmButton
                variant="destructive"
                warn="Delete this report and its logs?"
                onConfirm={() =>
                  remove.mutate(r.id, {
                    onSuccess: () => {
                      toast.success("Report deleted");
                      onClose();
                    },
                    onError: () => toast.error("Couldn't delete the report"),
                  })
                }
              >
                Delete
              </ConfirmButton>
            )}
            <Button onClick={() => void download()}>Download logs</Button>
          </>
        )
      }
    >
      <QueryState isLoading={report.isLoading} isError={report.isError} onRetry={() => void report.refetch()}>
        {r && (
          <div className="flex flex-col gap-4 text-sm">
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
              <Field label="Facility" value={r.user_artcc} />
              <Field label="App version" value={r.app_version} />
              <Field label="Platform" value={`${r.os} ${r.os_version} (${r.arch})`} />
              <Field label="Webview" value={r.webview_version} />
              <Field label="Window" value={r.window_label} />
              <Field label="Page" value={r.route} />
              <Field label="Logs" value={`${(r.logs_bytes / 1024).toFixed(1)} KB gzipped`} />
            </dl>
            <section>
              <h3 className="mb-1 font-semibold text-ink">Note</h3>
              <p className="whitespace-pre-wrap text-ink-2">{r.note || "—"}</p>
            </section>
            <section>
              <h3 className="mb-1 font-semibold text-ink">Details</h3>
              <pre className="max-h-80 overflow-auto rounded-lg border border-line-soft bg-ground p-3 font-mono text-xs text-ink-2">
                {JSON.stringify(r.meta, null, 2)}
              </pre>
            </section>
          </div>
        )}
      </QueryState>
    </Modal>
  );
}

function Field({ label, value }: { label: string; value?: string | null }) {
  return (
    <>
      <dt className="text-ink-3">{label}</dt>
      <dd className="font-mono text-xs text-ink">{value?.trim() || "—"}</dd>
    </>
  );
}

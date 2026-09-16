import {useMemo, useRef} from "react";
import {useNavigate, useParams} from "@tanstack/react-router";
import {Button, ConfirmButton, StatusPill, usePrompt, useToast} from "@ois/ui";
import {Share2, Trash2} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {Dashboard} from "@/features/dashboard/Dashboard";
import {
  useDashboard,
  useDeleteDashboard,
  useShareDashboard,
  useUnshareDashboard,
  useUpdateDashboard,
} from "@/lib/dashboards";

export function BoardViewPage() {
  const { boardId } = useParams({ from: "/ops/my/$boardId" });
  const { data: board } = useDashboard(boardId);
  const navigate = useNavigate();
  const prompt = usePrompt();
  const toast = useToast();
  const rename = useUpdateDashboard();
  const del = useDeleteDashboard();
  const share = useShareDashboard();
  const unshare = useUnshareDashboard();

  async function doRename() {
    const name = (
      await prompt({ title: "Rename dashboard", label: "Name", defaultValue: board?.name })
    )?.trim();
    if (name && name !== board?.name) rename.mutate({ id: boardId, name });
  }

  async function doShare() {
    try {
      const slug = await share.mutateAsync(boardId);
      const url = `${window.location.origin}/ops/my/shared/${slug}`;
      await navigator.clipboard.writeText(url);
      toast.success("Share link copied", { description: "Anyone signed in can open it." });
    } catch {
      toast.error("Couldn’t create a share link");
    }
  }

  async function doDelete() {
    await del.mutateAsync(boardId);
    navigate({ to: "/ops/my" });
  }

  // Header actions re-render only when the share state changes; handlers are read via a ref.
  const handlers = useRef({ doRename, doShare, doDelete, unshare: () => unshare.mutate(boardId) });
  handlers.current = { doRename, doShare, doDelete, unshare: () => unshare.mutate(boardId) };
  const shared = !!board?.share_slug;
  const actions = useMemo(
    () => (
      <div className="flex flex-wrap items-center gap-2">
        {shared && (
          <StatusPill tone="brand">
            <Share2 className="size-3" />
            Shared
          </StatusPill>
        )}
        <Button variant="outline" size="sm" onClick={() => void handlers.current.doRename()}>
          Rename
        </Button>
        <Button variant="outline" size="sm" onClick={() => void handlers.current.doShare()}>
          <Share2 />
          {shared ? "Copy link" : "Share"}
        </Button>
        {shared && (
          <Button variant="ghost" size="sm" onClick={() => handlers.current.unshare()}>
            Unshare
          </Button>
        )}
        <ConfirmButton
          size="icon"
          variant="ghost"
          className="shrink-0 text-ink-3 hover:text-danger"
          warn="Delete this board?"
          aria-label="Delete board"
          onConfirm={() => void handlers.current.doDelete()}
        >
          <Trash2 className="size-4" />
        </ConfirmButton>
      </div>
    ),
    [shared],
  );
  usePageHeader({ title: board?.name ?? "…", actions });

  return <Dashboard boardId={boardId} />;
}

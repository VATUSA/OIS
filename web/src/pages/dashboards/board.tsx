import {useNavigate, useParams} from "@tanstack/react-router";
import {Badge, Button, ConfirmButton, usePrompt, useToast} from "@ois/ui";
import {ChevronLeft, Share2, Trash2} from "lucide-react";

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

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="ghost" size="sm" onClick={() => navigate({ to: "/ops/my" })}>
          <ChevronLeft />
          Boards
        </Button>
        <h1 className="truncate text-xl font-semibold tracking-tight">{board?.name ?? "…"}</h1>
        {board?.share_slug && (
          <Badge variant="secondary" className="gap-1">
            <Share2 className="size-3" />
            Shared
          </Badge>
        )}
        <div className="ml-auto flex items-center gap-2">
          <Button variant="secondary" size="sm" onClick={doRename}>
            Rename
          </Button>
          <Button variant="secondary" size="sm" onClick={doShare}>
            <Share2 />
            {board?.share_slug ? "Copy link" : "Share"}
          </Button>
          {board?.share_slug && (
            <Button variant="ghost" size="sm" onClick={() => unshare.mutate(boardId)}>
              Unshare
            </Button>
          )}
          <ConfirmButton
            size="icon"
            variant="ghost"
            className="text-muted-foreground hover:text-destructive"
            warn="Delete this board?"
            onConfirm={doDelete}
          >
            <Trash2 className="size-4" />
          </ConfirmButton>
        </div>
      </div>

      <Dashboard boardId={boardId} />
    </div>
  );
}

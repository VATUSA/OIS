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
        <Button
          variant="ghost"
          size="sm"
          className="shrink-0"
          onClick={() => navigate({ to: "/ops/my" })}
        >
          <ChevronLeft />
          Boards
        </Button>
        <h1 className="min-w-0 flex-1 truncate text-xl font-semibold tracking-tight">
          {board?.name ?? "…"}
        </h1>
        {board?.share_slug && (
          <Badge variant="secondary" className="shrink-0 gap-1">
            <Share2 className="size-3" />
            Shared
          </Badge>
        )}
        {/* Full-width action row on phones, inline on ≥sm. */}
        <div className="flex w-full items-center gap-2 sm:w-auto">
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
            className="ml-auto shrink-0 text-muted-foreground hover:text-destructive sm:ml-0"
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

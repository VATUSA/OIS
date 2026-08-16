import {Link, useNavigate} from "@tanstack/react-router";
import {Badge, Button, Card, CardContent, ConfirmButton, usePrompt} from "@ois/ui";
import {LayoutDashboard, Plus, Share2, Trash2} from "lucide-react";

import {
  type DashboardSummary,
  useCreateDashboard,
  useDashboards,
  useDeleteDashboard,
  useUpdateDashboard,
} from "@/lib/dashboards";

function relativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const m = Math.round(diff / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

function BoardCard({
  board,
  onRename,
  onDelete,
}: {
  board: DashboardSummary;
  onRename: () => void;
  onDelete: () => void;
}) {
  return (
    <Card className="group relative transition-colors hover:border-primary/50">
      <CardContent className="flex flex-col gap-2 pt-5">
        <Link
          to="/ops/my/$boardId"
          params={{ boardId: board.id }}
          className="flex items-center gap-2 font-medium"
        >
          <LayoutDashboard className="size-4 text-muted-foreground" />
          <span className="truncate">{board.name}</span>
          {board.share_slug && (
            <Badge variant="secondary" className="ml-auto gap-1 text-xs">
              <Share2 className="size-3" />
              Shared
            </Badge>
          )}
        </Link>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span>Updated {relativeTime(board.updated_at)}</span>
          <div className="ml-auto flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
            <Button size="sm" variant="ghost" className="h-6 px-2" onClick={onRename}>
              Rename
            </Button>
            <ConfirmButton
              size="icon"
              variant="ghost"
              className="size-6 text-muted-foreground hover:text-destructive"
              warn={`Delete "${board.name}"?`}
              onConfirm={onDelete}
            >
              <Trash2 className="size-3.5" />
            </ConfirmButton>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function BoardLibraryPage() {
  const { data, isLoading } = useDashboards();
  const navigate = useNavigate();
  const prompt = usePrompt();
  const create = useCreateDashboard();
  const del = useDeleteDashboard();
  const rename = useUpdateDashboard();

  const boards = data?.dashboards ?? [];

  async function newBoard() {
    const name = (
      await prompt({
        title: "New dashboard",
        label: "Name",
        placeholder: "e.g. KJFK overview",
        confirmText: "Create",
      })
    )?.trim();
    if (!name) return;
    const board = await create.mutateAsync({ name });
    navigate({ to: "/ops/my/$boardId", params: { boardId: board.id } });
  }

  async function renameBoard(board: DashboardSummary) {
    const name = (
      await prompt({ title: "Rename dashboard", label: "Name", defaultValue: board.name })
    )?.trim();
    if (name && name !== board.name) rename.mutate({ id: board.id, name });
  }

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      <div className="flex items-center gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Dashboards</h1>
          <p className="text-muted-foreground">Your saved boards.</p>
        </div>
        <Button className="ml-auto" onClick={newBoard}>
          <Plus />
          New board
        </Button>
      </div>

      {isLoading ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading your boards…</p>
      ) : boards.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed py-20 text-center">
          <LayoutDashboard className="size-8 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">No dashboards yet.</p>
          <Button size="sm" onClick={newBoard}>
            <Plus />
            Create your first board
          </Button>
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {boards.map((b) => (
            <BoardCard
              key={b.id}
              board={b}
              onRename={() => void renameBoard(b)}
              onDelete={() => del.mutate(b.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

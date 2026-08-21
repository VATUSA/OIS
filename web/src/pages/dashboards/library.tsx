import {useState} from "react";
import {Link, useNavigate} from "@tanstack/react-router";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ConfirmButton,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  usePrompt,
} from "@ois/ui";
import {ChevronDown, Folder, FolderPlus, LayoutDashboard, Plus, Share2, Trash2} from "lucide-react";

import {FacilityCombobox, type FacilityPick} from "@/components/facility-combobox";
import {type Template, TEMPLATES} from "@/features/dashboard/templates";
import {
  type DashboardCollection,
  type DashboardSummary,
  useCreateCollection,
  useCreateDashboard,
  useDashboards,
  useDeleteCollection,
  useDeleteDashboard,
  useRenameCollection,
  useUpdateDashboard,
} from "@/lib/dashboards";

function relativeTime(iso: string): string {
  const m = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  return h < 24 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
}

function cleanIcaos(raw: string, one: boolean): string[] {
  const list = raw
    .split(/[,\s]+/)
    .map((s) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase())
    .filter((s) => s.length >= 3);
  return one ? list.slice(0, 1) : list;
}

function BoardCard({
  board,
  collections,
  onRename,
  onDelete,
  onMove,
}: {
  board: DashboardSummary;
  collections: DashboardCollection[];
  onRename: () => void;
  onDelete: () => void;
  onMove: (collectionId: string) => void;
}) {
  return (
    <Card className="group relative min-w-0 transition-colors hover:border-primary/50">
      <CardContent className="flex min-w-0 flex-col gap-2 pt-5">
        <Link
          to="/ops/my/$boardId"
          params={{ boardId: board.id }}
          className="flex min-w-0 items-center gap-2 font-medium"
        >
          <LayoutDashboard className="size-4 shrink-0 text-muted-foreground" />
          <span className="truncate">{board.name}</span>
          {board.share_slug && (
            <Badge variant="secondary" className="ml-auto shrink-0 gap-1 text-xs">
              <Share2 className="size-3" />
              Shared
            </Badge>
          )}
        </Link>
        <div className="flex items-center gap-1 text-xs text-muted-foreground">
          <span className="min-w-0 truncate">Updated {relativeTime(board.updated_at)}</span>
          {/* Always visible on touch (no hover); hover-reveal on pointer-fine screens. */}
          <div className="ml-auto flex shrink-0 items-center gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100">
            <Button size="sm" variant="ghost" className="h-7 px-2" onClick={onRename}>
              Rename
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="ghost" className="h-7 px-2">
                  Move
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => onMove("")}>No collection</DropdownMenuItem>
                {collections.length > 0 && <DropdownMenuSeparator />}
                {collections.map((c) => (
                  <DropdownMenuItem key={c.id} onSelect={() => onMove(c.id)}>
                    <Folder className="size-3.5" />
                    {c.name}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
            <ConfirmButton
              size="icon"
              variant="ghost"
              className="size-7 text-muted-foreground hover:text-destructive"
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
  const update = useUpdateDashboard();
  const createCollection = useCreateCollection();
  const renameCollection = useRenameCollection();
  const deleteCollection = useDeleteCollection();

  const boards = data?.dashboards ?? [];
  const collections = data?.collections ?? [];
  // A facility-scoped template waiting for the user to pick its facility.
  const [facTemplate, setFacTemplate] = useState<Template | null>(null);

  async function createBlank() {
    const name = (
      await prompt({
        title: "New dashboard",
        label: "Name",
        placeholder: "e.g. KJFK overview",
        confirmText: "Create",
      })
    )?.trim();
    if (!name) return;
    const b = await create.mutateAsync({ name });
    navigate({ to: "/ops/my/$boardId", params: { boardId: b.id } });
  }

  async function createFromTemplate(t: Template) {
    if (t.airports === "facility") {
      setFacTemplate(t); // opens the facility picker; completed in onFacilityTemplate
      return;
    }
    let icaos: string[] = [];
    if (t.airports !== "none") {
      const raw = await prompt({
        title: t.name,
        label: t.airports === "many" ? "Airports (comma-separated)" : "Airport (ICAO)",
        placeholder: t.airports === "many" ? "KJFK, KBOS, KLGA" : "KJFK",
        confirmText: "Create",
      });
      if (!raw) return;
      icaos = cleanIcaos(raw, t.airports === "one");
      if (icaos.length === 0) return;
    }
    const name = t.airports === "none" ? t.name : `${icaos.join("/")} · ${t.name}`;
    const b = await create.mutateAsync({ name, data: t.build({ icaos }) });
    navigate({ to: "/ops/my/$boardId", params: { boardId: b.id } });
  }

  async function onFacilityTemplate(pick: FacilityPick) {
    const t = facTemplate;
    setFacTemplate(null);
    if (!t) return;
    const b = await create.mutateAsync({
      name: `${pick.id} · ${t.name}`,
      data: t.build({ icaos: [], facility: pick }),
    });
    navigate({ to: "/ops/my/$boardId", params: { boardId: b.id } });
  }

  async function renameBoard(board: DashboardSummary) {
    const name = (
      await prompt({ title: "Rename dashboard", label: "Name", defaultValue: board.name })
    )?.trim();
    if (name && name !== board.name) update.mutate({ id: board.id, name });
  }

  async function newCollection() {
    const name = (
      await prompt({ title: "New collection", label: "Name", placeholder: "e.g. My ARTCC" })
    )?.trim();
    if (name) createCollection.mutate(name);
  }

  async function renameCol(c: DashboardCollection) {
    const name = (
      await prompt({ title: "Rename collection", label: "Name", defaultValue: c.name })
    )?.trim();
    if (name && name !== c.name) renameCollection.mutate({ id: c.id, name });
  }

  const cardOf = (b: DashboardSummary) => (
    <BoardCard
      key={b.id}
      board={b}
      collections={collections}
      onRename={() => void renameBoard(b)}
      onDelete={() => del.mutate(b.id)}
      onMove={(collectionId) => update.mutate({ id: b.id, collection_id: collectionId })}
    />
  );

  const ungrouped = boards.filter((b) => !b.collection_id);

  return (
    <div className="flex w-full flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Dashboards</h1>
          <p className="text-muted-foreground">Your saved boards.</p>
        </div>
        <div className="flex w-full items-center gap-2 sm:ml-auto sm:w-auto">
          <Button variant="secondary" className="flex-1 sm:flex-none" onClick={newCollection}>
            <FolderPlus />
            New collection
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button className="flex-1 sm:flex-none">
                <Plus />
                New board
                <ChevronDown className="size-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-64">
              <DropdownMenuItem onSelect={() => void createBlank()}>
                <LayoutDashboard />
                Blank board
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>From a template</DropdownMenuLabel>
              {TEMPLATES.map((t) => (
                <DropdownMenuItem
                  key={t.id}
                  className="flex-col items-start gap-0.5"
                  onSelect={() => void createFromTemplate(t)}
                >
                  <span className="font-medium">{t.name}</span>
                  <span className="text-xs text-muted-foreground">{t.description}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {isLoading ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading your boards…</p>
      ) : boards.length === 0 && collections.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed py-20 text-center">
          <LayoutDashboard className="size-8 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">No dashboards yet.</p>
          <Button size="sm" onClick={createBlank}>
            <Plus />
            Create your first board
          </Button>
        </div>
      ) : (
        <div className="flex flex-col gap-6">
          {collections.map((c) => {
            const items = boards.filter((b) => b.collection_id === c.id);
            return (
              <section key={c.id} className="flex flex-col gap-2">
                <div className="flex items-center gap-2 border-b pb-1">
                  <Folder className="size-4 shrink-0 text-muted-foreground" />
                  <h2 className="min-w-0 truncate text-sm font-semibold">{c.name}</h2>
                  <span className="shrink-0 text-xs text-muted-foreground">{items.length}</span>
                  <div className="ml-auto flex shrink-0 items-center gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2 text-xs"
                      onClick={() => void renameCol(c)}
                    >
                      Rename
                    </Button>
                    <ConfirmButton
                      size="icon"
                      variant="ghost"
                      className="size-7 text-muted-foreground hover:text-destructive"
                      warn={`Delete collection "${c.name}"? Boards move to Ungrouped.`}
                      onConfirm={() => deleteCollection.mutate(c.id)}
                    >
                      <Trash2 className="size-3.5" />
                    </ConfirmButton>
                  </div>
                </div>
                {items.length === 0 ? (
                  <p className="px-1 text-xs text-muted-foreground">
                    Empty — use a board’s “Move” menu to add one.
                  </p>
                ) : (
                  <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5">{items.map(cardOf)}</div>
                )}
              </section>
            );
          })}

          {ungrouped.length > 0 && (
            <section className="flex flex-col gap-2">
              {collections.length > 0 && (
                <h2 className="border-b pb-1 text-sm font-semibold text-muted-foreground">
                  Ungrouped
                </h2>
              )}
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5">
                {ungrouped.map(cardOf)}
              </div>
            </section>
          )}
        </div>
      )}

      {facTemplate && (
        <div
          className="fixed inset-0 z-[900] flex items-start justify-center bg-black/40 pt-32"
          onClick={() => setFacTemplate(null)}
        >
          <div className="w-80 rounded-lg border bg-background p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="mb-1 text-sm font-semibold">{facTemplate.name}</div>
            <p className="mb-3 text-xs text-muted-foreground">Pick an ARTCC or TRACON.</p>
            <FacilityCombobox autoFocus onSelect={onFacilityTemplate} />
          </div>
        </div>
      )}
    </div>
  );
}

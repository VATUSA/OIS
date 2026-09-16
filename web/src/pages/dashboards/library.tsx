import {useMemo, useRef, useState} from "react";
import {Link, useNavigate} from "@tanstack/react-router";
import {
  Button,
  Card,
  ConfirmButton,
  type DataColumn,
  DataTable,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  Modal,
  QueryState,
  StatusPill,
  usePrompt,
} from "@ois/ui";
import {Clock, ChevronDown, Folder, FolderPlus, LayoutDashboard, Plus, Share2, Trash2} from "lucide-react";

import {FacilityCombobox, type FacilityPick} from "@/components/facility-combobox";
import {usePageHeader, useView} from "@/components/shell/page-meta";
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

type BoardActions = {
  onRename: () => void;
  onDelete: () => void;
  onMove: (collectionId: string) => void;
};

function SharedPill() {
  return (
    <StatusPill tone="brand" className="shrink-0">
      <Share2 className="size-3" />
      Shared
    </StatusPill>
  );
}

/** Rename · Move · Delete for one board (shared by the grid card and the list row). */
function BoardMenu({
  board,
  collections,
  onRename,
  onDelete,
  onMove,
}: BoardActions & { board: DashboardSummary; collections: DashboardCollection[] }) {
  return (
    <>
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
        className="size-7 text-ink-3 hover:text-danger"
        warn={`Delete "${board.name}"?`}
        onConfirm={onDelete}
      >
        <Trash2 className="size-3.5" />
      </ConfirmButton>
    </>
  );
}

function BoardCard({
  board,
  collections,
  ...actions
}: BoardActions & { board: DashboardSummary; collections: DashboardCollection[] }) {
  return (
    <Card className="group relative flex min-w-0 flex-col gap-2 p-4 transition-colors hover:border-brand/50">
      <Link
        to="/ops/my/$boardId"
        params={{ boardId: board.id }}
        className="flex min-w-0 items-center gap-2 font-semibold"
      >
        <LayoutDashboard className="size-4 shrink-0 text-ink-3" />
        <span className="truncate">{board.name}</span>
        {board.share_slug && <span className="ml-auto"><SharedPill /></span>}
      </Link>
      <div className="flex items-center gap-1 text-xs text-ink-3">
        <span className="min-w-0 truncate">Updated {relativeTime(board.updated_at)}</span>
        {/* Always visible on touch (no hover); hover-reveal on pointer-fine screens. */}
        <div className="ml-auto flex shrink-0 items-center gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100">
          <BoardMenu board={board} collections={collections} {...actions} />
        </div>
      </div>
    </Card>
  );
}

const GRID = "grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5";

export function BoardLibraryPage() {
  const { data, isLoading, isError, refetch } = useDashboards();
  const navigate = useNavigate();
  const prompt = usePrompt();
  const view = useView();
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

  // The header actions are memoized once; they call the latest handlers through a ref.
  const handlers = useRef({ createBlank, createFromTemplate, newCollection });
  handlers.current = { createBlank, createFromTemplate, newCollection };
  const actions = useMemo(
    () => (
      <div className="flex items-center gap-2">
        <Button variant="outline" onClick={() => void handlers.current.newCollection()}>
          <FolderPlus />
          New collection
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button>
              <Plus />
              New board
              <ChevronDown className="size-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-64">
            <DropdownMenuItem onSelect={() => void handlers.current.createBlank()}>
              <LayoutDashboard />
              Blank board
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>From a template</DropdownMenuLabel>
            {TEMPLATES.map((t) => (
              <DropdownMenuItem
                key={t.id}
                className="flex-col items-start gap-0.5"
                onSelect={() => void handlers.current.createFromTemplate(t)}
              >
                <span className="font-semibold">{t.name}</span>
                <span className="text-xs text-ink-3">{t.description}</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    ),
    [],
  );
  usePageHeader({ subtitle: "Your saved boards.", count: data ? boards.length : null, actions });

  const actionsFor = (b: DashboardSummary): BoardActions => ({
    onRename: () => void renameBoard(b),
    onDelete: () => del.mutate(b.id),
    onMove: (collectionId) => update.mutate({ id: b.id, collection_id: collectionId }),
  });

  const cardOf = (b: DashboardSummary) => (
    <BoardCard key={b.id} board={b} collections={collections} {...actionsFor(b)} />
  );

  const ungrouped = boards.filter((b) => !b.collection_id);
  const collectionName = new Map(collections.map((c) => [c.id, c.name]));

  const columns: DataColumn<DashboardSummary>[] = [
    {
      accessorKey: "name",
      header: "Name",
      icon: LayoutDashboard,
      cell: (c) => (
        <Link
          to="/ops/my/$boardId"
          params={{ boardId: c.row.original.id }}
          className="font-semibold text-ink hover:text-brand-ink"
        >
          {c.getValue<string>()}
        </Link>
      ),
    },
    {
      id: "collection",
      accessorFn: (b) => (b.collection_id ? (collectionName.get(b.collection_id) ?? "") : ""),
      header: "Collection",
      icon: Folder,
      cell: (c) => <span className="text-ink-2">{c.getValue<string>() || "—"}</span>,
    },
    {
      accessorKey: "updated_at",
      header: "Updated",
      icon: Clock,
      mono: true,
      cell: (c) => <span className="whitespace-nowrap text-ink-2">{relativeTime(c.getValue<string>())}</span>,
    },
    {
      id: "shared",
      accessorFn: (b) => (b.share_slug ? 1 : 0),
      header: "Sharing",
      icon: Share2,
      cell: (c) => (c.row.original.share_slug ? <SharedPill /> : <span className="text-ink-3">Private</span>),
    },
    {
      id: "actions",
      header: "",
      enableSorting: false,
      align: "right",
      cell: (c) => (
        <div className="flex justify-end gap-1">
          <BoardMenu board={c.row.original} collections={collections} {...actionsFor(c.row.original)} />
        </div>
      ),
    },
  ];

  return (
    <div className="flex w-full flex-col gap-6">
      <QueryState isLoading={isLoading} isError={isError} onRetry={() => refetch()} loading="Loading your boards…">
        {boards.length === 0 && collections.length === 0 ? (
          <EmptyState
            icon={LayoutDashboard}
            className="rounded-md border border-line py-16"
            action={
              <Button size="sm" onClick={createBlank}>
                <Plus />
                Create your first board
              </Button>
            }
          >
            No dashboards yet.
          </EmptyState>
        ) : view === "list" ? (
          <DataTable
            label="Dashboards"
            columns={columns}
            data={boards}
            getRowId={(b) => b.id}
            initialSort={[{ id: "updated_at", desc: true }]}
            rowCap={25}
            empty="No boards yet — collections are empty."
          />
        ) : (
          <div className="flex flex-col gap-6">
            {collections.map((c) => {
              const items = boards.filter((b) => b.collection_id === c.id);
              return (
                <section key={c.id} className="flex flex-col gap-3">
                  <div className="flex items-center gap-2 border-b border-line pb-1.5">
                    <Folder className="size-4 shrink-0 text-ink-3" />
                    <h2 className="min-w-0 truncate text-sm font-semibold">{c.name}</h2>
                    <span className="shrink-0 font-mono text-xs text-ink-3">{items.length}</span>
                    <div className="ml-auto flex shrink-0 items-center gap-1">
                      <Button size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => void renameCol(c)}>
                        Rename
                      </Button>
                      <ConfirmButton
                        size="icon"
                        variant="ghost"
                        className="size-7 text-ink-3 hover:text-danger"
                        warn={`Delete collection "${c.name}"? Boards move to Ungrouped.`}
                        onConfirm={() => deleteCollection.mutate(c.id)}
                      >
                        <Trash2 className="size-3.5" />
                      </ConfirmButton>
                    </div>
                  </div>
                  {items.length === 0 ? (
                    <p className="px-1 text-xs text-ink-3">Empty — use a board’s “Move” menu to add one.</p>
                  ) : (
                    <div className={GRID}>{items.map(cardOf)}</div>
                  )}
                </section>
              );
            })}

            {ungrouped.length > 0 && (
              <section className="flex flex-col gap-3">
                {collections.length > 0 && (
                  <h2 className="border-b border-line pb-1.5 text-sm font-semibold text-ink-2">Ungrouped</h2>
                )}
                <div className={GRID}>{ungrouped.map(cardOf)}</div>
              </section>
            )}
          </div>
        )}
      </QueryState>

      <Modal
        open={facTemplate != null}
        onClose={() => setFacTemplate(null)}
        title={facTemplate?.name}
        description="Pick an ARTCC or TRACON."
        size="sm"
        placement="top"
      >
        <FacilityCombobox autoFocus onSelect={onFacilityTemplate} />
      </Modal>
    </div>
  );
}

import {useNavigate, useParams} from "@tanstack/react-router";
import {Button} from "@ois/ui";
import {Copy, LayoutDashboard} from "lucide-react";

import {DashboardGrid} from "@/features/dashboard/DashboardGrid";
import {type DashboardState, EMPTY_DASHBOARD} from "@/features/dashboard/types";
import {login, useMe} from "@/lib/auth";
import {useCopyDashboard, useSharedDashboard} from "@/lib/dashboards";

function coerce(raw: unknown): DashboardState {
  const s = raw as DashboardState | null | undefined;
  if (!s || s.version !== 1 || !Array.isArray(s.widgets) || !Array.isArray(s.layout)) {
    return EMPTY_DASHBOARD;
  }
  return s;
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-3 py-24 text-center text-sm text-muted-foreground">
      <LayoutDashboard className="size-8" />
      {children}
    </div>
  );
}

export function SharedBoardPage() {
  const { slug } = useParams({ from: "/ops/my/shared/$slug" });
  const { data: me, isLoading: meLoading } = useMe();
  const query = useSharedDashboard(me ? slug : null);
  const navigate = useNavigate();
  const copy = useCopyDashboard();

  async function saveCopy() {
    const id = await copy.mutateAsync(slug);
    navigate({ to: "/ops/my/$boardId", params: { boardId: id } });
  }

  if (meLoading) return <Centered>Loading…</Centered>;
  if (!me) {
    return (
      <Centered>
        <p>Sign in to view this shared dashboard.</p>
        <Button onClick={login}>Sign in with VATSIM</Button>
      </Centered>
    );
  }
  if (query.isError) return <Centered>This shared board wasn’t found.</Centered>;
  if (query.isLoading || !query.data) return <Centered>Loading shared board…</Centered>;

  const state = coerce(query.data.data);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <h1 className="truncate text-xl font-semibold tracking-tight">{query.data.name}</h1>
        <span className="text-sm text-muted-foreground">shared by {query.data.owner}</span>
        <Button className="ml-auto" onClick={saveCopy} disabled={copy.isPending}>
          <Copy />
          Save a copy
        </Button>
      </div>

      {state.widgets.length === 0 ? (
        <Centered>This board has no widgets.</Centered>
      ) : (
        <DashboardGrid
          state={state}
          editing={false}
          onLayoutChange={() => {}}
          onRemove={() => {}}
          onUpdate={() => {}}
        />
      )}
    </div>
  );
}

import {useMemo, useRef} from "react";
import {useNavigate, useParams} from "@tanstack/react-router";
import {Button, EmptyState} from "@ois/ui";
import {Copy, LayoutDashboard, Loader2} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
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

const Spinner = ({ className }: { className?: string }) => (
  <Loader2 className={"animate-spin " + (className ?? "")} />
);

function Centered({
  children,
  loading = false,
  action,
}: {
  children: React.ReactNode;
  loading?: boolean;
  action?: React.ReactNode;
}) {
  return (
    <EmptyState icon={loading ? Spinner : LayoutDashboard} className="py-24" action={action}>
      {children}
    </EmptyState>
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

  const saveRef = useRef(saveCopy);
  saveRef.current = saveCopy;
  const loaded = !!query.data;
  const pending = copy.isPending;
  const actions = useMemo(
    () =>
      loaded ? (
        <Button onClick={() => void saveRef.current()} disabled={pending}>
          <Copy />
          Save a copy
        </Button>
      ) : undefined,
    [loaded, pending],
  );
  usePageHeader({
    title: query.data?.name,
    subtitle: query.data ? `Shared by ${query.data.owner}` : undefined,
    actions,
  });

  if (meLoading) return <Centered loading>Loading…</Centered>;
  if (!me) {
    return (
      <Centered action={<Button onClick={login}>Sign in with VATSIM</Button>}>
        Sign in to view this shared dashboard.
      </Centered>
    );
  }
  if (query.isError) return <Centered>This shared board wasn’t found.</Centered>;
  if (query.isLoading || !query.data) return <Centered loading>Loading shared board…</Centered>;

  const state = coerce(query.data.data);

  return state.widgets.length === 0 ? (
    <Centered>This board has no widgets.</Centered>
  ) : (
    <DashboardGrid
      state={state}
      editing={false}
      onLayoutChange={() => {}}
      onRemove={() => {}}
      onUpdate={() => {}}
    />
  );
}

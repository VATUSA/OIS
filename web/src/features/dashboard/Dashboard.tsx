import {useState} from "react";
import {Button, EmptyState, QueryState} from "@ois/ui";
import {Check, LayoutDashboard, Pencil} from "lucide-react";

import {AddWidgetMenu} from "./AddWidgetMenu";
import {DashboardGrid} from "./DashboardGrid";
import {useBoardState} from "./useDashboardState";

function EmptyBoard({ editing, onStart }: { editing: boolean; onStart: () => void }) {
  return (
    <EmptyState
      icon={LayoutDashboard}
      className="rounded-md border border-line py-16"
      action={
        !editing && (
          <Button size="sm" onClick={onStart}>
            <Pencil />
            Customize
          </Button>
        )
      }
    >
      Your dashboard is empty. {editing ? "Use “Add widget” above." : "Add stat tiles and airport views."}
    </EmptyState>
  );
}

/** The editable widget grid for one board. The board name/actions live in BoardViewPage above. */
export function Dashboard({ boardId }: { boardId: string }) {
  const { state, loading, saving, addWidget, removeWidget, updateWidget, setLayout } =
    useBoardState(boardId);
  const [editing, setEditing] = useState(false);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        {saving && <span className="text-xs text-ink-3">saving…</span>}
        <div className="ml-auto flex items-center gap-2">
          {editing && <AddWidgetMenu onAdd={addWidget} />}
          <Button
            variant={editing ? "default" : "outline"}
            size="sm"
            onClick={() => setEditing((e) => !e)}
          >
            {editing ? (
              <>
                <Check />
                Done
              </>
            ) : (
              <>
                <Pencil />
                Edit
              </>
            )}
          </Button>
        </div>
      </div>

      {editing && (
        <p className="rounded-sm border border-line bg-panel-2 px-3 py-2 text-xs text-ink-2 sm:hidden">
          Widgets stack in one column on a phone. You can add, remove, and edit them here — drag
          &amp; resize to rearrange the layout on a larger screen.
        </p>
      )}

      {loading || !state ? (
        <QueryState isLoading loading="Loading your dashboard…" className="py-16" />
      ) : state.widgets.length === 0 ? (
        <EmptyBoard editing={editing} onStart={() => setEditing(true)} />
      ) : (
        <DashboardGrid
          boardId={boardId}
          state={state}
          editing={editing}
          onLayoutChange={setLayout}
          onRemove={removeWidget}
          onUpdate={updateWidget}
        />
      )}
    </div>
  );
}

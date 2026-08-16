import {useState} from "react";
import {Button} from "@ois/ui";
import {Check, LayoutDashboard, Pencil} from "lucide-react";

import {AddWidgetMenu} from "./AddWidgetMenu";
import {DashboardGrid} from "./DashboardGrid";
import {useDashboardState} from "./useDashboardState";

function EmptyState({ editing, onStart }: { editing: boolean; onStart: () => void }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed py-20 text-center">
      <LayoutDashboard className="size-8 text-muted-foreground" />
      <p className="text-sm text-muted-foreground">
        Your dashboard is empty.{" "}
        {editing ? "Use “Add widget” above." : "Add stat tiles and airport views."}
      </p>
      {!editing && (
        <Button size="sm" onClick={onStart}>
          <Pencil />
          Customize
        </Button>
      )}
    </div>
  );
}

export function Dashboard() {
  const { state, loading, saving, addWidget, removeWidget, setLayout } = useDashboardState();
  const [editing, setEditing] = useState(false);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">My dashboard</h1>
          <p className="text-muted-foreground">
            Your personal, customizable view.{saving ? " · saving…" : ""}
          </p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          {editing && <AddWidgetMenu onAdd={addWidget} />}
          <Button
            variant={editing ? "default" : "secondary"}
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

      {loading || !state ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading your dashboard…</p>
      ) : state.widgets.length === 0 ? (
        <EmptyState editing={editing} onStart={() => setEditing(true)} />
      ) : (
        <DashboardGrid
          state={state}
          editing={editing}
          onLayoutChange={setLayout}
          onRemove={removeWidget}
        />
      )}
    </div>
  );
}

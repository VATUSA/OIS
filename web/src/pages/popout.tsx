import {useEffect, useState} from "react";
import {useParams} from "@tanstack/react-router";
import {EmptyState, QueryState} from "@ois/ui";
import {PanelsTopLeft} from "lucide-react";

import {useDashboard} from "@/lib/dashboards";
import {normalize} from "@/features/dashboard/useDashboardState";
import {useFcaTraffic, useFcas} from "@/lib/fca";
import {WidgetBody} from "@/features/dashboard/render";
import {Ladder} from "@/pages/fca/ladder";

/**
 * The pages a pop-out mini-window shows (#349).
 *
 * Opened with `?embed=1`, which `RootLayout` already uses to render without the shell — no sidebar,
 * no breadcrumbs, no alerts — so these are just the panel, filling the window.
 *
 * They deliberately fetch for themselves rather than sharing the main window's cache: a Tauri
 * window is its own webview, so there is no cache to share. Both windows mount the same realtime
 * socket and hit the same endpoints, so they stay in step because the server is the source of truth.
 */

/** A second ticking on its own, so the ladder moves even when no data has changed. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** One dashboard widget, floated out of its board. */
export function PopoutWidgetPage() {
  const {boardId, widgetId} = useParams({from: "/popout/widget/$boardId/$widgetId"});
  const board = useDashboard(boardId);
  const widget = normalize(board.data?.data).widgets.find((w) => w.id === widgetId);

  return (
    <div className="h-full w-full overflow-auto p-2">
      <QueryState isLoading={board.isLoading} isError={board.isError}>
        {widget ? (
          <WidgetBody widget={widget} editing={false} onUpdate={() => undefined} />
        ) : (
          // The widget was removed from the board while its window was open.
          <EmptyState icon={PanelsTopLeft}>This panel is no longer on the board.</EmptyState>
        )}
      </QueryState>
    </div>
  );
}

/** The FCA metering ladder, floated over whatever else the controller is running. */
export function PopoutFcaLadderPage() {
  const {fcaId} = useParams({from: "/popout/fca/$fcaId"});
  const now = useNow();
  const fcas = useFcas();
  const traffic = useFcaTraffic(fcaId);
  const fca = fcas.data?.find((f) => f.id === fcaId);

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <div className="shrink-0 border-b border-line px-3 py-2 text-xs font-semibold text-ink">
        {fca?.name ?? "FCA"}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        <QueryState isLoading={traffic.isLoading} isError={traffic.isError}>
          <Ladder flights={traffic.data ?? []} now={now} />
        </QueryState>
      </div>
    </div>
  );
}

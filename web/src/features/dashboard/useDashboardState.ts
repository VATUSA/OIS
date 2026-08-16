import {useCallback, useEffect, useRef, useState} from "react";

import {useDashboard, useUpdateDashboard} from "@/lib/dashboards";

import {defaultCell, EMPTY_DASHBOARD, type DashboardState, type GridCell, type Widget} from "./types";

const SAVE_DEBOUNCE_MS = 800;

/** Coerce whatever the server returned into a valid DashboardState. */
function normalize(raw: unknown): DashboardState {
  const s = raw as DashboardState | null | undefined;
  if (!s || s.version !== 1 || !Array.isArray(s.widgets) || !Array.isArray(s.layout)) {
    return EMPTY_DASHBOARD;
  }
  return s;
}

/** True if two layouts place every cell identically (position + size). Ignores minW/minH, which the
 * grid re-derives and which never move a widget. Used to drop no-op onLayoutChange events. */
function sameGeometry(a: GridCell[], b: GridCell[]): boolean {
  if (a.length !== b.length) return false;
  const prev = new Map(a.map((c) => [c.i, c]));
  for (const c of b) {
    const p = prev.get(c.i);
    if (!p || p.x !== c.x || p.y !== c.y || p.w !== c.w || p.h !== c.h) return false;
  }
  return true;
}

/**
 * Loads one board's DashboardState from the server, holds it as local state, and persists changes
 * (debounced PUT). Returns the state plus mutations; every write goes through `update`.
 */
export function useBoardState(boardId: string) {
  const query = useDashboard(boardId);
  const save = useUpdateDashboard();
  const [state, setState] = useState<DashboardState | null>(null);

  // Seed local state once the board first arrives. Reset when the board id changes.
  useEffect(() => {
    setState(null);
  }, [boardId]);
  useEffect(() => {
    if (query.isLoading || !query.data) return;
    setState((prev) => prev ?? normalize(query.data.data));
  }, [query.isLoading, query.data]);

  // Debounced persist; keep the latest pending value so we can flush on unmount.
  const saveRef = useRef(save);
  saveRef.current = save;
  const idRef = useRef(boardId);
  idRef.current = boardId;
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<DashboardState | null>(null);

  const doSave = useCallback((next: DashboardState) => {
    saveRef.current.mutate({ id: idRef.current, data: next });
  }, []);

  const flush = useCallback(() => {
    if (timer.current !== undefined) {
      window.clearTimeout(timer.current);
      timer.current = undefined;
    }
    if (pending.current) {
      doSave(pending.current);
      pending.current = null;
    }
  }, [doSave]);

  const persist = useCallback(
    (next: DashboardState) => {
      pending.current = next;
      if (timer.current !== undefined) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        timer.current = undefined;
        if (pending.current) {
          doSave(pending.current);
          pending.current = null;
        }
      }, SAVE_DEBOUNCE_MS);
    },
    [doSave],
  );

  useEffect(() => flush, [flush]); // flush any pending save when the page unmounts

  const update = useCallback(
    (updater: (s: DashboardState) => DashboardState) => {
      setState((prev) => {
        const next = updater(prev ?? EMPTY_DASHBOARD);
        persist(next);
        return next;
      });
    },
    [persist],
  );

  const addWidget = useCallback(
    (widget: Widget) => {
      update((s) => {
        const maxY = s.layout.reduce((m, c) => Math.max(m, c.y + c.h), 0);
        const size = defaultCell(widget);
        const cell: GridCell = { i: widget.id, x: 0, y: maxY, ...size };
        return { ...s, widgets: [...s.widgets, widget], layout: [...s.layout, cell] };
      });
    },
    [update],
  );

  const removeWidget = useCallback(
    (id: string) => {
      update((s) => ({
        ...s,
        widgets: s.widgets.filter((w) => w.id !== id),
        layout: s.layout.filter((c) => c.i !== id),
      }));
    },
    [update],
  );

  /** Patch one widget's config (e.g. a table's columns/sort). Patch is widget-shape-specific. */
  const updateWidget = useCallback(
    (id: string, patch: Record<string, unknown>) => {
      update((s) => ({
        ...s,
        widgets: s.widgets.map((w) => (w.id === id ? ({ ...w, ...patch } as Widget) : w)),
      }));
    },
    [update],
  );

  const setLayout = useCallback(
    (cells: GridCell[]) => {
      // react-grid-layout calls onLayoutChange on every internal reflow (mount, width changes,
      // re-render), not just real drags/resizes. If we blindly produced a new state each time, that
      // state → new layout prop → RGL reflow → onLayoutChange → … loops forever and freezes the tab
      // once there are enough widgets to keep the geometry churning. So bail when nothing actually
      // moved: returning the SAME state reference makes React skip the re-render, ending the cycle.
      setState((prev) => {
        if (!prev) return prev;
        const nextLayout: GridCell[] = cells.map((c) => ({
          i: c.i,
          x: c.x,
          y: c.y,
          w: c.w,
          h: c.h,
          minW: c.minW,
          minH: c.minH,
        }));
        if (sameGeometry(prev.layout, nextLayout)) return prev;
        const next = { ...prev, layout: nextLayout };
        persist(next);
        return next;
      });
    },
    [persist],
  );

  return {
    state,
    loading: query.isLoading && state == null,
    saving: save.isPending,
    addWidget,
    removeWidget,
    updateWidget,
    setLayout,
  };
}

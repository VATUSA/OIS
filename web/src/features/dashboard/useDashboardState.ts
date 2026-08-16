import {useCallback, useEffect, useRef, useState} from "react";

import {usePreferences, useSavePreferences} from "@/lib/preferences";

import {defaultCell, EMPTY_DASHBOARD, type DashboardState, type GridCell, type Widget} from "./types";

const NS = "dashboard";
const SAVE_DEBOUNCE_MS = 800;

/** Coerce whatever the server returned into a valid DashboardState. */
function normalize(raw: DashboardState | null | undefined): DashboardState {
  if (!raw || raw.version !== 1 || !Array.isArray(raw.widgets) || !Array.isArray(raw.layout)) {
    return EMPTY_DASHBOARD;
  }
  return raw;
}

/**
 * Loads the user's dashboard from server prefs, holds it as local state, and persists changes
 * (debounced). Returns the state plus mutations; all writes go through `update` so every change
 * is saved consistently.
 */
export function useDashboardState() {
  const query = usePreferences<DashboardState>(NS);
  const save = useSavePreferences<DashboardState>(NS);
  const [state, setState] = useState<DashboardState | null>(null);

  // Seed local state once the server value (or null) first arrives.
  useEffect(() => {
    if (query.isLoading) return;
    setState((prev) => prev ?? normalize(query.data));
  }, [query.isLoading, query.data]);

  // Debounced persist; keep the latest pending value so we can flush on unmount.
  const saveRef = useRef(save);
  saveRef.current = save;
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<DashboardState | null>(null);

  const flush = useCallback(() => {
    if (timer.current !== undefined) {
      window.clearTimeout(timer.current);
      timer.current = undefined;
    }
    if (pending.current) {
      saveRef.current.mutate(pending.current);
      pending.current = null;
    }
  }, []);

  const persist = useCallback((next: DashboardState) => {
    pending.current = next;
    if (timer.current !== undefined) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = undefined;
      if (pending.current) {
        saveRef.current.mutate(pending.current);
        pending.current = null;
      }
    }, SAVE_DEBOUNCE_MS);
  }, []);

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
        const size = defaultCell(widget.kind);
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
      update((s) => ({
        ...s,
        layout: cells.map((c) => ({
          i: c.i,
          x: c.x,
          y: c.y,
          w: c.w,
          h: c.h,
          minW: c.minW,
          minH: c.minH,
        })),
      }));
    },
    [update],
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

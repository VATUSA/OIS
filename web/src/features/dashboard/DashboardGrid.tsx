import {ResponsiveGridLayout, useContainerWidth, type Layout} from "react-grid-layout";
import "react-grid-layout/css/styles.css";

import {WidgetBody, widgetTitle} from "./render";
import {defaultCell, GRID_COLS, type DashboardState, type GridCell, type Widget} from "./types";
import {WidgetFrame} from "./WidgetFrame";

const ROW_HEIGHT = 64;
const MARGIN = 12;
/** Below this container width the grid collapses to a single stacked column (phones). */
const MOBILE_BREAKPOINT = 640;

/** Presentational widgets render without the standard header chrome. */
const isBare = (k: Widget["kind"]) => k === "text" || k === "divider";
/** Map/chart bodies fill their cell edge-to-edge. */
const isFlush = (k: Widget["kind"]) => k === "map" || k === "chart";

/**
 * The react-grid-layout surface. We use a single breakpoint (always "lg") so there is exactly one
 * stored layout array — the column *width* still scales fluidly with the container via
 * useContainerWidth. Drag/resize are gated by `editing`; drag is limited to the `.widget-drag-handle`
 * so the widget body stays interactive. On narrow containers the grid is bypassed for a plain
 * stacked column (drag/resize don't make sense on a phone).
 */
export function DashboardGrid({
  state,
  editing,
  onLayoutChange,
  onRemove,
  onUpdate,
}: {
  state: DashboardState;
  editing: boolean;
  onLayoutChange: (layout: GridCell[]) => void;
  onRemove: (id: string) => void;
  onUpdate: (id: string, patch: Record<string, unknown>) => void;
}) {
  const { width, containerRef } = useContainerWidth();
  const mobile = width > 0 && width < MOBILE_BREAKPOINT;

  const frameFor = (w: Widget, draggable: boolean) => (
    <WidgetFrame
      title={widgetTitle(w)}
      editing={editing}
      draggable={draggable}
      bare={isBare(w.kind)}
      flush={isFlush(w.kind)}
      onRemove={() => onRemove(w.id)}
    >
      <WidgetBody widget={w} editing={editing} onUpdate={onUpdate} />
    </WidgetFrame>
  );

  // One layout item per rendered widget; re-apply per-kind min sizes so a corrupt/old stored
  // cell can never resize below its minimum.
  const layout: Layout = state.widgets.map((w, idx) => {
    const size = defaultCell(w);
    const stored = state.layout.find((c) => c.i === w.id);
    return stored
      ? { ...stored, minW: size.minW, minH: size.minH }
      : { i: w.id, x: 0, y: idx, w: size.w, h: size.h, minW: size.minW, minH: size.minH };
  });

  if (mobile) {
    // Preserve the desktop reading order (top-to-bottom, left-to-right) in the stack.
    const cellOf = (id: string) => layout.find((c) => c.i === id);
    const ordered = [...state.widgets].sort((a, b) => {
      const ca = cellOf(a.id);
      const cb = cellOf(b.id);
      return (ca?.y ?? 0) - (cb?.y ?? 0) || (ca?.x ?? 0) - (cb?.x ?? 0);
    });
    return (
      <div ref={containerRef} className="flex flex-col gap-3">
        {ordered.map((w) => {
          const h = cellOf(w.id)?.h ?? defaultCell(w).h;
          return (
            <div key={w.id} style={{ height: h * ROW_HEIGHT + (h - 1) * MARGIN }}>
              {frameFor(w, false)}
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div ref={containerRef}>
      {width > 0 && (
        <ResponsiveGridLayout
          width={width}
          layouts={{ lg: layout }}
          breakpoints={{ lg: 0 }}
          cols={{ lg: GRID_COLS }}
          rowHeight={ROW_HEIGHT}
          margin={[MARGIN, MARGIN]}
          containerPadding={[0, 0]}
          dragConfig={{ enabled: editing, handle: ".widget-drag-handle" }}
          resizeConfig={{ enabled: editing }}
          onLayoutChange={(current) => onLayoutChange(current as GridCell[])}
        >
          {state.widgets.map((w) => (
            <div key={w.id}>{frameFor(w, true)}</div>
          ))}
        </ResponsiveGridLayout>
      )}
    </div>
  );
}

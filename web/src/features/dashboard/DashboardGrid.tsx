import {ResponsiveGridLayout, useContainerWidth, type Layout} from "react-grid-layout";
import "react-grid-layout/css/styles.css";

import {WidgetBody, widgetTitle} from "./render";
import {defaultCell, GRID_COLS, type DashboardState, type GridCell} from "./types";
import {WidgetFrame} from "./WidgetFrame";

const ROW_HEIGHT = 64;

/**
 * The react-grid-layout surface. We use a single breakpoint (always "lg") so there is exactly one
 * stored layout array — the column *width* still scales fluidly with the container via
 * useContainerWidth. Drag/resize are gated by `editing`; drag is limited to the `.widget-drag-handle`
 * so the widget body stays interactive.
 */
export function DashboardGrid({
  state,
  editing,
  onLayoutChange,
  onRemove,
}: {
  state: DashboardState;
  editing: boolean;
  onLayoutChange: (layout: GridCell[]) => void;
  onRemove: (id: string) => void;
}) {
  const { width, containerRef } = useContainerWidth();

  // One layout item per rendered widget; re-apply per-kind min sizes so a corrupt/old stored
  // cell can never resize below its minimum.
  const layout: Layout = state.widgets.map((w, idx) => {
    const size = defaultCell(w.kind);
    const stored = state.layout.find((c) => c.i === w.id);
    return stored
      ? { ...stored, minW: size.minW, minH: size.minH }
      : { i: w.id, x: 0, y: idx, w: size.w, h: size.h, minW: size.minW, minH: size.minH };
  });

  return (
    <div ref={containerRef}>
      {width > 0 && (
        <ResponsiveGridLayout
          width={width}
          layouts={{ lg: layout }}
          breakpoints={{ lg: 0 }}
          cols={{ lg: GRID_COLS }}
          rowHeight={ROW_HEIGHT}
          margin={[12, 12]}
          containerPadding={[0, 0]}
          dragConfig={{ enabled: editing, handle: ".widget-drag-handle" }}
          resizeConfig={{ enabled: editing }}
          onLayoutChange={(current) => onLayoutChange(current as GridCell[])}
        >
          {state.widgets.map((w) => (
            <div key={w.id}>
              <WidgetFrame
                title={widgetTitle(w)}
                editing={editing}
                flush={w.kind === "map"}
                onRemove={() => onRemove(w.id)}
              >
                <WidgetBody widget={w} />
              </WidgetFrame>
            </div>
          ))}
        </ResponsiveGridLayout>
      )}
    </div>
  );
}

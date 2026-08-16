import {FcaMap} from "@/components/fca-map";

/**
 * The flow map as a dashboard widget — read-only and embedded (fills the cell, no sidebar).
 * FcaMap already watches its container with a ResizeObserver and calls invalidateSize, so it
 * repaints correctly when the grid cell is dragged/resized.
 */
export function MapWidgetView({ initialFlight }: { initialFlight?: string }) {
  return <FcaMap readOnly embedded initialFlight={initialFlight} />;
}

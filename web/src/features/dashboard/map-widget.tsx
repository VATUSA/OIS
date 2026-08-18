import {FcaMap} from "@/components/fca-map";

/**
 * The flow map as a dashboard widget — read-only and embedded (fills the cell, no sidebar).
 * FcaMap already watches its container with a ResizeObserver and calls invalidateSize, so it
 * repaints correctly when the grid cell is dragged/resized.
 *
 * Inside a HistoricalProvider the map's traffic + ATC layers reconstruct at the scrubber instant
 * (useTraffic / useAtc are mode-aware), so the widget replays the network at T. FCA overlays remain
 * current config (config history is out of scope).
 */
export function MapWidgetView({ initialFlight }: { initialFlight?: string }) {
  return <FcaMap readOnly embedded initialFlight={initialFlight} />;
}

import {FcaMap} from "@/components/fca-map";
import {useHistoricalAt} from "@/lib/historical-context";

/**
 * The flow map as a dashboard widget — read-only and embedded (fills the cell, no sidebar).
 * FcaMap already watches its container with a ResizeObserver and calls invalidateSize, so it
 * repaints correctly when the grid cell is dragged/resized.
 *
 * The embedded map still draws the live network, so in historical replay it would contradict the
 * scrubber — show a placeholder instead (a reconstructed map widget is a later phase; the standalone
 * capture replay at /stats/captures/{id}/replay already covers a historical map today).
 */
export function MapWidgetView({ initialFlight }: { initialFlight?: string }) {
  if (useHistoricalAt() != null) {
    return (
      <div className="flex h-full items-center justify-center p-4 text-center text-sm text-muted-foreground">
        The map widget shows live traffic — use the capture replay for a historical map.
      </div>
    );
  }
  return <FcaMap readOnly embedded initialFlight={initialFlight} />;
}

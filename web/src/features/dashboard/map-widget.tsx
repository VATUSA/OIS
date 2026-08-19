import {FcaMapView} from "@/components/map/FcaMapView";

/**
 * The flow map as a dashboard widget — read-only and embedded (fills the cell, no sidebar). Now on the
 * shared deck.gl TrafficMap (FcaMapView); it repaints on resize via deck's own observer.
 *
 * Inside a HistoricalProvider the map's traffic + ATC layers reconstruct at the scrubber instant
 * (useTraffic / useAtc are mode-aware), so the widget replays the network at T. FCA overlays remain
 * current config (config history is out of scope).
 */
export function MapWidgetView({ initialFlight }: { initialFlight?: string }) {
  return <FcaMapView readOnly embedded initialFlight={initialFlight} />;
}

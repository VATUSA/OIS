import {useSearch} from "@tanstack/react-router";

import {FcaMapView} from "@/components/map/FcaMapView";

/** Controller FCA flow tool (`/ops/fca`) — the full, editable map on the shared deck.gl TrafficMap. */
export function FcaPage() {
  // `?fca=<id>` selects that FCA on arrival — where a desktop release/metering notification lands.
  const initialFcaId = useSearch({ from: "/ops/fca" }).fca;
  return <FcaMapView persistKey="ops-fca" initialFcaId={initialFcaId} />;
}

import {useSearch} from "@tanstack/react-router";

import {FcaMapView} from "@/components/map/FcaMapView";

/**
 * Public FCA overview (`/advisories/fcas`) — the same map as the controller tool, in read-only mode:
 * no create/edit/delete affordances. A `?flight=` param (from the pilot page) auto-locates that flight.
 */
export function AdvisoriesFcaPage() {
  const { flight } = useSearch({ strict: false }) as { flight?: string };
  return <FcaMapView readOnly initialFlight={flight} persistKey="advisories" />;
}

import {useSearch} from "@tanstack/react-router";

import {FcaMap} from "@/components/fca-map";

/**
 * Public FCA overview (`/advisories/fcas`) — the exact same map as the
 * controller tool, in read-only mode: no create/edit/delete affordances.
 * A `?flight=` param (from the pilot page) auto-locates that flight.
 */
export function AdvisoriesFcaPage() {
  const { flight } = useSearch({ strict: false }) as { flight?: string };
  return <FcaMap readOnly initialFlight={flight} />;
}

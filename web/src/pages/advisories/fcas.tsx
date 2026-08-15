import {FcaMap} from "@/components/fca-map";

/**
 * Public FCA overview (`/advisories/fcas`) — the exact same map as the
 * controller tool, in read-only mode: no create/edit/delete affordances.
 */
export function AdvisoriesFcaPage() {
  return <FcaMap readOnly />;
}

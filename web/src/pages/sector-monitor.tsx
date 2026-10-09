import {useState} from "react";
import {EmptyState, FilterBar, QueryState, Select} from "@ois/ui";
import {Building2, Lock} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {FacilityDemand} from "@/features/sector-demand/SectorDemand";
import {useFacilities} from "@/lib/admin";
import {type Me, useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";

const SUBTITLE =
  "Predicted aircraft per sector in each 15-minute bin, against its limit: yellow while holding departures can still prevent it, red once airborne traffic alone exceeds it.";

/** The read the page's one endpoint is gated on, and its nav item's permission. */
const SECTOR_DEMAND_READ = "flow.sectors.read";

function SectorMonitor({ me }: { me: Me | null | undefined }) {
  const facilities = useFacilities();
  // Chosen in this session only. A remembered pick would outlive a move between facilities, which
  // #725 rules out; the default is the viewer's home facility.
  const [picked, setPicked] = useState<string | null>(null);

  const artccs = (facilities.data ?? [])
    .filter((f) => f.active)
    .map((f) => f.id)
    .sort();
  const home = me?.vatusa?.home_facility?.toUpperCase() ?? "";
  const artcc = picked ?? (artccs.includes(home) ? home : "");

  return (
    <QueryState
      isLoading={facilities.isLoading}
      isError={facilities.isError}
      onRetry={() => void facilities.refetch()}
      className="rounded-md border border-line"
    >
      <div className="flex min-w-0 flex-col gap-4">
        <FilterBar>
          <Select aria-label="Facility" size="sm" value={artcc} onChange={(e) => setPicked(e.target.value)}>
            <option value="">Select a facility…</option>
            {artccs.map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </Select>
        </FilterBar>
        {artcc ? (
          <FacilityDemand key={artcc} artcc={artcc} />
        ) : (
          <EmptyState icon={Building2} className="rounded-md border border-line">
            Pick a facility to see its sector demand.
          </EmptyState>
        )}
      </div>
    </QueryState>
  );
}

/**
 * The Operations page for sector demand (#725): the selected facility's enroute and TRACON tables,
 * then its neighbours, collapsed and view-only. The set follows the facility selector and nothing
 * else. The body below the selector is drawn as vTBFM's Sector Monitor (#794, a named DESIGN.md
 * exception); the selector and the shell around it stay OIS.
 */
export function SectorMonitorPage() {
  const { data: me } = useMe();
  usePageHeader({ subtitle: SUBTITLE });
  if (!hasPermission(me, SECTOR_DEMAND_READ)) {
    return (
      <EmptyState icon={Lock} className="rounded-md border border-line">
        You don&apos;t have access to sector demand.
      </EmptyState>
    );
  }
  return <SectorMonitor me={me} />;
}

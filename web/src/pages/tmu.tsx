import {useNavigate, useSearch} from "@tanstack/react-router";
import {EmptyState, Tabs} from "@ois/ui";
import {ShieldAlert} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {TMU_TABS, type TmuTab as Tab} from "@/lib/tmu-tabs";
import {AdvisoriesTab} from "@/pages/tmu/advisories";
import {GdpTab} from "@/pages/tmu/gdp";
import {GroundStopsTab} from "@/pages/tmu/ground-stops";
import {ProgramsTab} from "@/pages/tmu/programs";
import {RateCalculatorTab} from "@/pages/tmu/rate-calc";
import {RestrictionsTab} from "@/pages/tmu/restrictions";

/**
 * The page title, naming the active tab. It is also what ⌘⇧F stores as a favorite's label, and
 * `?tab=` is part of the favorite's key — so without the tab in the title, a favorite per tab read
 * as a column of identical "TMU" rows (VATUSA/OIS#339).
 */
export const tmuTitle = (tabLabel: string | undefined) => (tabLabel ? `TMU · ${tabLabel}` : "TMU");

export function TmuPage() {
  const { data: me } = useMe();
  const tabs = TMU_TABS.filter((t) => hasPermission(me, t.permission));

  const { tab: requestedTab, facility } = useSearch({ strict: false }) as { tab?: Tab; facility?: string };
  const navigate = useNavigate();
  const active = tabs.some((t) => t.value === requestedTab) ? requestedTab : tabs[0]?.value;

  // The board/table view switch only applies to the Programs tab.
  usePageHeader({
    title: tmuTitle(tabs.find((t) => t.value === active)?.label),
    subtitle: "Airport rate programs and inter-facility restrictions.",
    views: active === "programs" ? undefined : null,
  });

  function selectTab(id: Tab) {
    void navigate({
      to: "/ops/tmu",
      search: (prev) => ({ ...prev, tab: id }),
      replace: true,
      resetScroll: false,
    });
  }

  if (tabs.length === 0) {
    return (
      <EmptyState icon={ShieldAlert} className="h-full">
        You don&apos;t have traffic-management access.
      </EmptyState>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      {tabs.length > 1 && active && (
        <Tabs value={active} onChange={selectTab} items={tabs} />
      )}

      {active === "programs" && <ProgramsTab />}
      {/* Keyed on `?facility=` so arriving from ⌘K while already here re-seeds the filters. */}
      {active === "restrictions" && <RestrictionsTab key={facility ?? ""} />}
      {active === "ground-stops" && <GroundStopsTab />}
      {active === "gdp" && <GdpTab />}
      {active === "rate-calculator" && <RateCalculatorTab />}
      {active === "advisories" && <AdvisoriesTab />}
    </div>
  );
}

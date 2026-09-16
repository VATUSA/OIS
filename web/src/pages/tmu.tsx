import {useNavigate, useSearch} from "@tanstack/react-router";
import {EmptyState, Tabs} from "@ois/ui";
import {Calculator, Clock, Gauge, OctagonPause, ShieldAlert} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {GdpTab} from "@/pages/tmu/gdp";
import {GroundStopsTab} from "@/pages/tmu/ground-stops";
import {ProgramsTab} from "@/pages/tmu/programs";
import {RateCalculatorTab} from "@/pages/tmu/rate-calc";
import {RestrictionsTab} from "@/pages/tmu/restrictions";

type Tab =
  | "programs"
  | "restrictions"
  | "ground-stops"
  | "gdp"
  | "rate-calculator";

type TabDef = { value: Tab; label: string; icon: typeof Gauge };

export function TmuPage() {
  const { data: me } = useMe();
  const canPrograms = hasPermission(me, "tmu.program.read");
  const canRestrictions = hasPermission(me, "tmu.tmi.read");
  const canGroundStops = hasPermission(me, "tmu.groundstop.read");
  const canGdp = hasPermission(me, "tmu.gdp.read");

  const tabs = [
    canPrograms && { value: "programs", label: "Programs", icon: Gauge },
    canRestrictions && { value: "restrictions", label: "Restrictions", icon: ShieldAlert },
    canGroundStops && { value: "ground-stops", label: "Ground stops", icon: OctagonPause },
    canGdp && { value: "gdp", label: "Ground delay", icon: Clock },
    canPrograms && { value: "rate-calculator", label: "Rate calculator", icon: Calculator },
  ].filter(Boolean) as TabDef[];

  const { tab: requestedTab } = useSearch({ strict: false }) as { tab?: Tab };
  const navigate = useNavigate();
  const active = tabs.some((t) => t.value === requestedTab) ? requestedTab : tabs[0]?.value;

  // The board/table view switch only applies to the Programs tab.
  usePageHeader({
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
      {active === "restrictions" && <RestrictionsTab />}
      {active === "ground-stops" && <GroundStopsTab />}
      {active === "gdp" && <GdpTab />}
      {active === "rate-calculator" && <RateCalculatorTab />}
    </div>
  );
}

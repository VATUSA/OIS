import {useEffect} from "react";
import {useNavigate, useSearch} from "@tanstack/react-router";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {GdpTab} from "@/pages/tmu/gdp";
import {GroundStopsTab} from "@/pages/tmu/ground-stops";
import {ProgramsTab} from "@/pages/tmu/programs";
import {RateCalculatorTab} from "@/pages/tmu/rate-calc";
import {RestrictionsTab} from "@/pages/tmu/restrictions";
import {resolveTmuTab, saveLastTmuTab, type Tab} from "@/pages/tmu/tab-state";

export function TmuPage() {
  const { data: me } = useMe();
  const canPrograms = hasPermission(me, "tmu.program.read");
  const canRestrictions = hasPermission(me, "tmu.tmi.read");
  const canGroundStops = hasPermission(me, "tmu.groundstop.read");
  const canGdp = hasPermission(me, "tmu.gdp.read");

  const tabs: { id: Tab; label: string }[] = [
    canPrograms && { id: "programs" as const, label: "Programs" },
    canRestrictions && { id: "restrictions" as const, label: "Restrictions" },
    canGroundStops && { id: "ground-stops" as const, label: "Ground stops" },
    canGdp && { id: "gdp" as const, label: "Ground delay" },
    canPrograms && { id: "rate-calculator" as const, label: "Rate calculator" },
  ].filter(Boolean) as { id: Tab; label: string }[];

  const { tab: requestedTab } = useSearch({ strict: false }) as { tab?: Tab };
  const navigate = useNavigate();
  const { active, needsUrlSync } = resolveTmuTab(tabs, requestedTab);

  useEffect(() => {
    if (active) saveLastTmuTab(active);
  }, [active]);

  useEffect(() => {
    if (!needsUrlSync || !active) return;
    void navigate({
      to: "/ops/tmu",
      search: (prev) => ({ ...prev, tab: active }),
      replace: true,
      resetScroll: false,
    });
  }, [needsUrlSync, active, navigate]);

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
      <div className="flex h-[calc(100vh-3.5rem)] items-center justify-center text-sm text-muted-foreground">
        You don&apos;t have traffic-management access.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          Traffic Management
        </h1>
        <p className="text-muted-foreground">
          Airport rate programs and inter-facility restrictions.
        </p>
      </div>

      {tabs.length > 1 && (
        <div className="-mx-4 flex gap-1 overflow-x-auto border-b px-4 sm:mx-0 sm:px-0">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => selectTab(t.id)}
              className={
                "-mb-px shrink-0 whitespace-nowrap border-b-2 px-4 py-2 text-sm font-medium transition-colors " +
                (active === t.id
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground")
              }
            >
              {t.label}
            </button>
          ))}
        </div>
      )}

      {active === "programs" && <ProgramsTab />}
      {active === "restrictions" && <RestrictionsTab />}
      {active === "ground-stops" && <GroundStopsTab />}
      {active === "gdp" && <GdpTab />}
      {active === "rate-calculator" && <RateCalculatorTab />}
    </div>
  );
}

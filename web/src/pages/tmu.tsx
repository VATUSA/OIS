import {useState} from "react";

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

  const [tab, setTab] = useState<Tab>(tabs[0]?.id ?? "programs");
  const active = tabs.some((t) => t.id === tab) ? tab : tabs[0]?.id;

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
              onClick={() => setTab(t.id)}
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

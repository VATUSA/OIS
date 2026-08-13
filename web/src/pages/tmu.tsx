import {useState} from "react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {ProgramsTab} from "@/pages/tmu/programs";
import {RestrictionsTab} from "@/pages/tmu/restrictions";

type Tab = "programs" | "restrictions";

export function TmuPage() {
  const { data: me } = useMe();
  const canPrograms = hasPermission(me, "tmu.program.read");
  const canRestrictions = hasPermission(me, "tmu.tmi.read");

  const tabs: { id: Tab; label: string }[] = [
    canPrograms && { id: "programs" as const, label: "Programs" },
    canRestrictions && { id: "restrictions" as const, label: "Restrictions" },
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
        <div className="flex gap-1 border-b">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              className={
                "-mb-px border-b-2 px-4 py-2 text-sm font-medium transition-colors " +
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

      {active === "programs" ? <ProgramsTab /> : <RestrictionsTab />}
    </div>
  );
}

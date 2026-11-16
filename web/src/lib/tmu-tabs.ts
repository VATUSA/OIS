import {Calculator, Clock, FileText, Gauge, type LucideIcon, OctagonPause, ShieldAlert} from "lucide-react";

export type TmuTab = "programs" | "restrictions" | "ground-stops" | "gdp" | "rate-calculator" | "advisories";

export type TmuTabDef = { value: TmuTab; label: string; icon: LucideIcon; permission: string };

/**
 * The TMU page's tabs in display order, each with the read permission that admits it. The page's
 * tabs and the Operations nav link both derive from this list, so the link shows exactly to users
 * who will find a tab behind it (#966).
 */
export const TMU_TABS: readonly TmuTabDef[] = [
  { value: "programs", label: "Programs", icon: Gauge, permission: "tmu.program.read" },
  { value: "restrictions", label: "Restrictions", icon: ShieldAlert, permission: "tmu.tmi.read" },
  { value: "ground-stops", label: "Ground stops", icon: OctagonPause, permission: "tmu.groundstop.read" },
  { value: "gdp", label: "Ground delay", icon: Clock, permission: "tmu.gdp.read" },
  { value: "rate-calculator", label: "Rate calculator", icon: Calculator, permission: "tmu.program.read" },
  { value: "advisories", label: "Advisories", icon: FileText, permission: "tmu.adv.read" },
];

/** Each TMU tab's permission, in tab order (Programs and Rate calculator share one). */
export const TMU_TAB_PERMISSIONS: readonly string[] = TMU_TABS.map((t) => t.permission);

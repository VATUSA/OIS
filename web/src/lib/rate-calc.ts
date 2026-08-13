// vatflow's TMU rate calculator, ported. Recommended AAR = max AAR × (on-duty / full
// staffing), optionally capped by the mix of open positions.

export const TIERS = {
  small: { label: "Small", maxAar: 20, fullStaff: 2 },
  medium: { label: "Medium", maxAar: 45, fullStaff: 4 },
  large: { label: "Large", maxAar: 80, fullStaff: 5 },
  hub: { label: "Hub", maxAar: 120, fullStaff: 6 },
} as const;

export type TierKey = keyof typeof TIERS;

export const POSITIONS = [
  { key: "app", label: "Approach (APP)", weight: 0.4 },
  { key: "twr", label: "Tower (TWR)", weight: 0.35 },
  { key: "gnd", label: "Ground (GND)", weight: 0.12 },
  { key: "del", label: "Delivery (DEL)", weight: 0.05 },
  { key: "sup", label: "Coordinator (SUP)", weight: 0.05 },
  { key: "atis", label: "ATIS / other", weight: 0.03 },
] as const;

export type PositionKey = (typeof POSITIONS)[number]["key"];
export type Positions = Record<PositionKey, boolean>;

const HUB = new Set([
  "KATL", "KORD", "KLAX", "KJFK", "KDFW", "KMIA", "KPHX", "KCLT", "KSEA", "KSFO",
]);
const LARGE = new Set([
  "KDEN", "KIAH", "KEWR", "KMCO", "KLAS", "KBOS", "KDTW", "KMSP", "KPHL",
]);
const MEDIUM = new Set([
  "KDCA", "KBWI", "KSLC", "KSAN", "KPDX", "KSTL", "KAUS", "KBNA", "KRDU",
]);

/** Best-guess tier for a known airport ICAO (falls back to "small"). */
export function tierForIcao(code: string): TierKey {
  if (HUB.has(code)) return "hub";
  if (LARGE.has(code)) return "large";
  if (MEDIUM.has(code)) return "medium";
  return "small";
}

export type CalcInput = {
  maxAar: number;
  fullStaff: number;
  onDuty: number;
  usePositions: boolean;
  positions: Positions;
};

export type LimitedBy = "headcount" | "positions" | "critical" | "staff";

export type CalcResult = {
  aar: number;
  /** Recommended AAR as a percentage of max capacity. */
  pct: number;
  limitedBy: LimitedBy;
  warning: string | null;
};

export function recommendedAar(input: CalcInput): CalcResult {
  const max = Math.max(1, Math.round(input.maxAar) || 1);
  const full = Math.max(1, Math.round(input.fullStaff) || 1);
  const duty = Math.max(0, Math.round(input.onDuty) || 0);

  if (duty < 1) {
    return { aar: 0, pct: 0, limitedBy: "staff", warning: "No controllers on duty" };
  }

  let factor = Math.min(1, duty / full); // exponent = 1
  let limitedBy: LimitedBy = "headcount";

  if (input.usePositions) {
    let posFactor = 0;
    for (const p of POSITIONS) if (input.positions[p.key]) posFactor += p.weight;
    if (factor > posFactor) limitedBy = "positions";
    factor = Math.min(factor, posFactor);
    // Without APP or TWR open, capacity is critically limited.
    if (!input.positions.app && !input.positions.twr && factor > 0.15) {
      factor = 0.15;
      limitedBy = "critical";
    }
  }

  let aar = Math.round(max * factor);
  let warning: string | null = null;
  if (aar < 1) {
    aar = 0;
    warning = "Insufficient staffing for IFR arrivals";
  } else {
    aar = Math.min(200, Math.max(1, aar));
  }
  return { aar, pct: Math.round(factor * 100), limitedBy, warning };
}

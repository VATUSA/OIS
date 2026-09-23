import type {Tone} from "@ois/ui";

/**
 * Every domain status → `StatusPill` tone, in one place. Pages render
 * `<StatusPill tone={toneOf("publish", s)}>{label}</StatusPill>` instead of each keeping its own
 * status→variant function. Unknown statuses fall back to `neutral`.
 */
const TONES = {
  /** TMI / ground stop / GDP lifecycle. */
  publish: { draft: "neutral", published: "good", expired: "neutral", cancelled: "bad" },
  /** Event DCC coordination. */
  dcc: { not_needed: "neutral", requested: "warn", confirmed: "good" },
  /** ACE staffing requests. */
  ace: { open: "brand", completed: "neutral", cancelled: "bad" },
  /** A recorded flight session (historical). */
  session: { active: "good", completed: "neutral" },
  /** Event review workflow. */
  review: { pending: "warn", approved: "good", rejected: "bad", denied: "bad" },
  /** Event traffic recording. */
  recording: { recording: "good", scheduled: "brand", recorded: "neutral" },
  /** An FCA's event lifecycle. */
  eventFca: { planned: "brand", published: "good", archived: "neutral" },
  /** Facility support level for an event. */
  support: { required: "good", preferred: "brand", not_required: "neutral" },
  /** Live flight state (airport, AADC, FCA). */
  flight: { airborne: "airborne", ground: "ground", proposed: "proposed", arrived: "arrived" },
  /** Load level (runway bins, GDP, delays). */
  level: { green: "good", yellow: "warn", red: "bad" },
  /** Flight category. */
  category: { VFR: "vfr", MVFR: "mvfr", IFR: "ifr", LIFR: "lifr" },
  /** Event TMI package lifecycle. */
  tmiPackage: { draft: "neutral", activated: "good", archived: "neutral" },
  /** A controller's event availability. */
  availability: { available: "good", partial: "warn", unavailable: "bad" },
  /** API key state. */
  apiKey: { active: "good", disabled: "neutral", expired: "bad" },
} as const satisfies Record<string, Record<string, Tone>>;

export type StatusKind = keyof typeof TONES;

export function toneOf(kind: StatusKind, status: string | null | undefined): Tone {
  if (!status) return "neutral";
  return (TONES[kind] as Record<string, Tone>)[status] ?? "neutral";
}

/** Short labels for live flight state (AIR / GND / PROP / ARR). */
export const FLIGHT_STATE_LABEL: Record<string, string> = {
  airborne: "AIR",
  ground: "GND",
  proposed: "PROP",
  arrived: "ARR",
};

/** An audit action's tone, from its verb. */
/**
 * A crossing flight's state tone, label and CSS colour (unknown states read as ground).
 *
 * Lives here rather than beside the ladder because both the FCA strip list and the metering ladder
 * need it, and the ladder is now rendered from two places (the detail panel and a pop-out window).
 */
export function flightStatus(s: string): { tone: Tone; label: string; color: string } {
  const known = toneOf("flight", s) !== "neutral";
  const state = known ? s : "ground";
  return {
    tone: toneOf("flight", state),
    label: FLIGHT_STATE_LABEL[state]!,
    color: `var(--flight-${state})`,
  };
}

/** An audit action's tone, from its verb. */
export function auditActionTone(action: string): Tone {
  const a = action.toLowerCase();
  if (/(create|assign|grant|add|publish|activat)/.test(a)) return "good";
  if (/(delete|revoke|disable|remove|deny|cancel)/.test(a)) return "bad";
  return "neutral";
}

/** A background job's current state as a tone + label. */
export function jobState(job: { running: boolean; last_ok?: boolean | null }): { tone: Tone; label: string } {
  if (job.running) return { tone: "brand", label: "Running…" };
  if (job.last_ok == null) return { tone: "neutral", label: "Never run" };
  return job.last_ok ? { tone: "good", label: "OK" } : { tone: "bad", label: "Failed" };
}

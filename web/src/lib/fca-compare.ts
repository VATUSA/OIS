import type {FcaFlight} from "@/lib/fca";

/** Why one FCA flight crosses ahead of another (#819). */
export type OrderReason = "manual" | "eta" | "spacing";

type Ordered = Pick<FcaFlight, "callsign" | "eta" | "manually_placed">;

function etaMs(f: Ordered): number {
  return f.eta ? Date.parse(f.eta) : Number.NaN;
}

/**
 * The reason `ahead` (the lower sequence number) crosses before `behind`.
 *
 * Two flights the controller placed are in that order because the controller put them there. A
 * flight that would reach the line first by ETA but crosses second was moved behind: by the
 * controller when either flight's position is manual, otherwise by the scheduler holding spacing
 * behind a pinned crossing (an airborne flight or an issued release). Anything else is ETA order.
 */
export function orderReason(ahead: Ordered, behind: Ordered): OrderReason {
  if (ahead.manually_placed && behind.manually_placed) return "manual";
  if (etaMs(behind) < etaMs(ahead)) {
    return ahead.manually_placed || behind.manually_placed ? "manual" : "spacing";
  }
  return "eta";
}

/** One sentence for the compare panel. */
export function orderReasonText(reason: OrderReason, ahead: string, behind: string): string {
  switch (reason) {
    case "manual":
      return `${ahead} is ahead of ${behind} by manual placement.`;
    case "spacing":
      return `${behind} has the earlier ETA but was pushed behind ${ahead} to hold spacing.`;
    case "eta":
      return `${ahead} is ahead of ${behind} in ETA order.`;
  }
}

/** Seconds from `ahead`'s metered crossing to `behind`'s; null when either has no crossing time. */
export function pairGapSec(
  ahead: Pick<FcaFlight, "cross_time">,
  behind: Pick<FcaFlight, "cross_time">,
): number | null {
  if (!ahead.cross_time || !behind.cross_time) return null;
  return Math.round((Date.parse(behind.cross_time) - Date.parse(ahead.cross_time)) / 1000);
}

/** A gap as `m:ss · N.Nnm`; negative when the flight behind crosses first (a release out of order). */
export function fmtGap(sec: number, nm: number): string {
  const sign = sec < 0 ? "-" : "";
  const abs = Math.abs(sec);
  const mmss = `${Math.floor(abs / 60)}:${String(abs % 60).padStart(2, "0")}`;
  return `${sign}${mmss} · ${nm.toFixed(1)}nm`;
}

/** Seconds as distance at `speedKt`, nm to one decimal (the backend's own conversion). */
export function nmAt(sec: number, speedKt: number): number {
  return Math.round((sec / 3600) * speedKt * 10) / 10;
}

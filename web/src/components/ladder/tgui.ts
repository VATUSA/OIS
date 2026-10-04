// The TGUI arrival ladder's pure parts (VATUSA/OIS#557): the delay label, the clock ticks, and the
// shape a column is handed in. Modelled on vTBFM's TGUI from its manual's behavioural description
// only — vTBFM is not open source and none of its code is used.
//
// Nothing here schedules, spaces or predicts. Every time a column draws comes from a field the
// metering engine already publishes; the only arithmetic is presentation (rounding and banding a
// delay the server already computed).

import {hhmmZulu} from "@/lib/time";

/** One flight in a TGUI column: where it sits on each rail and how its tags are labelled. */
export interface TguiItem {
  key: string;
  callsign: string;
  /** The ETA rail (left): minutes from now, and its instant. `null` when the flight has no ETA. */
  etaMin: number | null;
  etaTime: string | null;
  /** The schedule rail (right): exactly the time the classic ladder plots, so the two renderings
   * show one sequence (#557 AC5). */
  staMin: number;
  staTime: string;
  /** The delay the engine absorbed, in minutes — `null` when there is no schedule to be late
   * against, in which case no label is drawn rather than a delay being invented. */
  delayMin: number | null;
  /** The time is committed — a release (FCA) or an issued CFR (airport). OIS has no freeze horizon,
   * so this is what stands in for vTBFM's "frozen". */
  committed: boolean;
  /** Wake category (`L`/`M`/`H`/`J`), when the stream carries one. */
  wake?: string | null;
}

export interface TguiColumn {
  id: string;
  name: string;
  /** What the column's reference is: an FCA line, or a meter fix (an airport's arrival gate). */
  kind: "FCA" | "MFX";
  items: TguiItem[];
}

export type DelayLevel = "ok" | "watch" | "over" | "early";

/**
 * The delay label beside a schedule tag (§7.7 of the manual): `STA − ETA`, rounded, two digits.
 *
 * | rounded delay | label  | level |
 * | ------------- | ------ | ----- |
 * | 0             | none   |       |
 * | 1–5           | 01–05  | ok    |
 * | 6–14          | 06–14  | watch |
 * | 15–99         | 15–99  | over  |
 * | ≥ 100         | ++     | over  |
 * | ≤ −1 (early)  | -N     | early |
 *
 * The manual colours 11–14 orange; OIS's level palette (DESIGN.md) has no orange step, so that band
 * shares `watch`. Its label is unchanged.
 */
export function delayLabel(min: number | null | undefined): { text: string; level: DelayLevel } | null {
  if (min == null || !Number.isFinite(min)) return null;
  const r = Math.round(min);
  if (r === 0) return null; // also covers -0, from Math.round(-0.5)
  if (r < 0) return { text: `-${-r}`, level: "early" };
  if (r >= 100) return { text: "++", level: "over" };
  const text = String(r).padStart(2, "0");
  if (r <= 5) return { text, level: "ok" };
  if (r <= 14) return { text, level: "watch" };
  return { text, level: "over" };
}

export interface Tick {
  /** Minutes from now. */
  min: number;
  /** A five-minute tick is longer and carries the Zulu time. */
  major: boolean;
  /** `HHMM` (no `z`), only on a major tick. */
  label: string | null;
}

/** A tick on every clock minute from now to `now + win`. Ticks sit on real clock minutes rather than
 * on offsets from now, so the axis moves continuously with the clock. */
export function tickMarks(now: number, win: number): Tick[] {
  const ticks: Tick[] = [];
  for (let t = Math.ceil(now / 60_000) * 60_000; (t - now) / 60_000 <= win; t += 60_000) {
    const major = new Date(t).getUTCMinutes() % 5 === 0;
    ticks.push({
      min: (t - now) / 60_000,
      major,
      label: major ? hhmmZulu(new Date(t).toISOString()).slice(0, 4) : null,
    });
  }
  return ticks;
}

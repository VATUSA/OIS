import * as React from "react";

import {cn} from "../lib/utils";

/** Every status the UI can express. Semantic tones first, then aviation domain tones. */
export type Tone =
  | "good"
  | "warn"
  | "bad"
  | "brand"
  | "neutral"
  | "airborne"
  | "ground"
  | "proposed"
  | "arrived"
  | "vfr"
  | "mvfr"
  | "ifr"
  | "lifr";

/** Text colour per tone — also useful on its own (a coloured count, a dot). */
export const toneText: Record<Tone, string> = {
  good: "text-success",
  warn: "text-warning",
  bad: "text-danger",
  brand: "text-brand-ink",
  neutral: "text-ink-2",
  airborne: "text-flight-airborne",
  ground: "text-flight-ground",
  proposed: "text-flight-proposed",
  arrived: "text-flight-arrived",
  vfr: "text-cat-vfr",
  mvfr: "text-cat-mvfr",
  ifr: "text-cat-ifr",
  lifr: "text-cat-lifr",
};

const toneFill: Record<Tone, string> = {
  good: "bg-success-soft",
  warn: "bg-warning-soft",
  bad: "bg-danger-soft",
  brand: "bg-brand-soft",
  neutral: "bg-chip",
  airborne: "bg-flight-airborne/15",
  ground: "bg-flight-ground/15",
  proposed: "bg-flight-proposed/15",
  arrived: "bg-flight-arrived/15",
  vfr: "bg-cat-vfr/15",
  mvfr: "bg-cat-mvfr/15",
  ifr: "bg-cat-ifr/15",
  lifr: "bg-cat-lifr/15",
};

/** Background colour per tone for a solid swatch or dot. */
export const toneBg: Record<Tone, string> = {
  good: "bg-success",
  warn: "bg-warning",
  bad: "bg-danger",
  brand: "bg-brand",
  neutral: "bg-ink-3",
  airborne: "bg-flight-airborne",
  ground: "bg-flight-ground",
  proposed: "bg-flight-proposed",
  arrived: "bg-flight-arrived",
  vfr: "bg-cat-vfr",
  mvfr: "bg-cat-mvfr",
  ifr: "bg-cat-ifr",
  lifr: "bg-cat-lifr",
};

/**
 * A status pill: soft tint + same-hue text. `dot` adds a leading dot (people/liveness status). Status
 * only — never an action (DESIGN.md).
 */
export function StatusPill({
  tone = "neutral",
  dot = false,
  className,
  children,
}: {
  tone?: Tone;
  dot?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-0.5 text-xs font-semibold leading-5",
        toneFill[tone],
        toneText[tone],
        className,
      )}
    >
      {dot && <span className="size-1.5 rounded-full bg-current" aria-hidden="true" />}
      {children}
    </span>
  );
}

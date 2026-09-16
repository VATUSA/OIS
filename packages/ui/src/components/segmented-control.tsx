import * as React from "react";
import type {LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";

export type SegmentOption<T extends string> = { value: T; label: React.ReactNode; icon?: LucideIcon };

/**
 * A segmented control: the active option is a filled inner tile, the rest quiet text. Use for a
 * small, mutually exclusive choice (a view switch, Arrivals/Departures, a bucket size).
 */
export function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  size = "md",
  className,
  "aria-label": ariaLabel,
}: {
  value: T;
  onChange: (value: T) => void;
  options: readonly SegmentOption<T>[];
  size?: "sm" | "md";
  className?: string;
  "aria-label"?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      className={cn("inline-flex shrink-0 rounded-sm border border-line bg-panel-2 p-0.5", className)}
    >
      {options.map((o) => {
        const on = o.value === value;
        const Icon = o.icon;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={on}
            onClick={() => onChange(o.value)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-xs font-semibold transition-colors",
              size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-xs",
              on ? "bg-card text-ink" : "text-ink-3 hover:text-ink-2",
            )}
          >
            {Icon && <Icon className="size-3.5" />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

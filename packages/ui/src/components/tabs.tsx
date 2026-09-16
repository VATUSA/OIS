import * as React from "react";
import type {LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";

export type TabItem<T extends string> = {
  value: T;
  label: React.ReactNode;
  icon?: LucideIcon;
  /** Optional mono count shown after the label. */
  count?: number;
};

/**
 * Text tabs — quiet labels, the active one on a filled `--panel-2` tile. For switching between the
 * sections of one page (TMU programs / TMIs, an event's tabs). For a two-to-four-way view choice use
 * `SegmentedControl` instead.
 */
export function Tabs<T extends string>({
  value,
  onChange,
  items,
  className,
}: {
  value: T;
  onChange: (value: T) => void;
  items: readonly TabItem<T>[];
  className?: string;
}) {
  return (
    <div role="tablist" className={cn("flex flex-wrap gap-0.5", className)}>
      {items.map((t) => {
        const on = t.value === value;
        const Icon = t.icon;
        return (
          <button
            key={t.value}
            type="button"
            role="tab"
            aria-selected={on}
            onClick={() => onChange(t.value)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-sm px-3 py-1.5 text-sm font-semibold transition-colors",
              on ? "bg-panel-2 text-ink" : "text-ink-3 hover:text-ink-2",
            )}
          >
            {Icon && <Icon className={cn("size-4", on && "text-brand-ink")} />}
            {t.label}
            {t.count != null && <span className="font-mono text-xs text-ink-3">{t.count}</span>}
          </button>
        );
      })}
    </div>
  );
}

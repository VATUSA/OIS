import * as React from "react";
import type {LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {SegmentedControl, type SegmentOption} from "./segmented-control";

/**
 * The page header (DESIGN.md "Wayfinding"): a large 700 title with a neutral count chip, a subtitle,
 * and on the right an optional view switch and page actions. The shell renders it from route meta.
 */
export function PageHeader<V extends string = string>({
  title,
  icon: Icon,
  count,
  subtitle,
  views,
  view,
  onViewChange,
  actions,
  className,
}: {
  title: React.ReactNode;
  icon?: LucideIcon;
  count?: number | null;
  subtitle?: React.ReactNode;
  views?: readonly SegmentOption<V>[];
  view?: V;
  onViewChange?: (view: V) => void;
  actions?: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-wrap items-start justify-between gap-3", className)}>
      <div className="min-w-0">
        <div className="flex items-center gap-2.5">
          {Icon && <Icon className="size-5 shrink-0 text-ink-3" />}
          <h1 className="truncate text-[22px] font-bold leading-tight tracking-tight text-ink sm:text-[28px]">
            {title}
          </h1>
          {count != null && (
            <span className="rounded-full bg-panel-2 px-2.5 py-0.5 font-mono text-xs font-semibold text-ink-2">
              {count}
            </span>
          )}
        </div>
        {subtitle && <p className="mt-1 text-sm text-ink-2">{subtitle}</p>}
      </div>
      {(actions || (views && view && onViewChange)) && (
        <div className="flex flex-wrap items-center gap-2">
          {views && view && onViewChange && (
            <SegmentedControl aria-label="View" value={view} onChange={onViewChange} options={views} />
          )}
          {actions}
        </div>
      )}
    </div>
  );
}

import * as React from "react";
import {ChevronDown, Plus, X, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";

/**
 * A filter chip: pill, leading icon, label, and either a chevron (opens a picker — wrap it in a
 * `DropdownMenuTrigger asChild`) or, when `onClear` is set, a clear ✕. `active` marks a chip whose
 * filter currently narrows the data.
 */
export const FilterChip = React.forwardRef<
  HTMLButtonElement,
  React.ButtonHTMLAttributes<HTMLButtonElement> & {
    icon?: LucideIcon;
    label: React.ReactNode;
    /** The selected value, shown after the label. */
    value?: React.ReactNode;
    active?: boolean;
    onClear?: () => void;
  }
>(({ icon: Icon, label, value, active, onClear, className, ...props }, ref) => (
  <span
    className={cn(
      "inline-flex h-8 items-center rounded-full border text-xs",
      active ? "border-brand/40 bg-brand-soft text-ink" : "border-line bg-panel-2 text-ink-2",
      className,
    )}
  >
    <button
      ref={ref}
      type="button"
      className="inline-flex h-full items-center gap-1.5 rounded-full pl-3 pr-2 hover:text-ink"
      {...props}
    >
      {Icon && <Icon className="size-3.5 text-ink-3" />}
      <span>{label}</span>
      {value != null && <span className="font-semibold text-ink">{value}</span>}
      {!onClear && <ChevronDown className="size-3.5 text-ink-3" />}
    </button>
    {onClear && (
      <button
        type="button"
        aria-label="Clear filter"
        onClick={onClear}
        className="mr-1.5 rounded-full p-0.5 text-ink-3 hover:text-ink"
      >
        <X className="size-3.5" />
      </button>
    )}
  </span>
));
FilterChip.displayName = "FilterChip";

/** The ghost "+ Add filter" affordance that sits after a row of chips. */
export const AddFilter = React.forwardRef<HTMLButtonElement, React.ButtonHTMLAttributes<HTMLButtonElement>>(
  ({ className, children = "Add filter", ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      className={cn("inline-flex h-8 items-center gap-1.5 px-2 text-xs text-ink-3 hover:text-ink-2", className)}
      {...props}
    >
      <Plus className="size-3.5" />
      {children}
    </button>
  ),
);
AddFilter.displayName = "AddFilter";

/** Lays out a filter row: chips wrap, trailing content (search, reset) sits at the end. */
export function FilterBar({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("flex flex-wrap items-center gap-2", className)}>{children}</div>;
}

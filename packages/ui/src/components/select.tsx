import * as React from "react";
import {ChevronDown} from "lucide-react";

import {cn} from "../lib/utils";

/**
 * A styled native `<select>` — native keeps keyboard, mobile pickers and forms working for free.
 * Pass `<option>`s as children.
 */
export const Select = React.forwardRef<
  HTMLSelectElement,
  Omit<React.SelectHTMLAttributes<HTMLSelectElement>, "size"> & { size?: "sm" | "md"; wrapperClassName?: string }
>(({ className, wrapperClassName, size = "md", children, ...props }, ref) => (
  <span className={cn("relative inline-flex", wrapperClassName)}>
    <select
      ref={ref}
      className={cn(
        "w-full appearance-none rounded-xs border border-line bg-panel-2 pl-2.5 pr-7 text-sm text-ink transition-colors",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
        size === "sm" ? "h-8" : "h-9",
        className,
      )}
      {...props}
    >
      {children}
    </select>
    <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-ink-3" />
  </span>
));
Select.displayName = "Select";

import * as React from "react";
import {Slot} from "@radix-ui/react-slot";
import {ChevronRight, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";

/**
 * The console shell (DESIGN.md "The shell"): a top bar over one rounded container on `--ground`
 * that holds an optional collapsible sidebar and the main area. The main area has a breadcrumb row;
 * with a sidebar, the content's top-left corner curves where the two dividers meet. The only shadow
 * in the system sits under this container.
 */
export function Shell({
  topBar,
  sidebar,
  breadcrumbs,
  children,
  className,
}: {
  topBar: React.ReactNode;
  /** A `<Sidebar>`; omit for pages outside an area (home, account). */
  sidebar?: React.ReactNode;
  breadcrumbs?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-dvh flex-col bg-ground text-ink", className)}>
      {topBar}
      <div className="flex min-h-0 flex-1 px-2 pb-2 sm:px-3 sm:pb-3">
        <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-panel shadow-[0_30px_80px_-40px_rgb(0_0_0/0.9)]">
          {sidebar}
          <div className="flex min-w-0 flex-1 flex-col">
            {breadcrumbs && (
              <div className={cn("flex h-11 shrink-0 items-center px-5", sidebar && "border-l border-line")}>
                {breadcrumbs}
              </div>
            )}
            <div
              className={cn(
                "relative flex min-h-0 flex-1 flex-col",
                breadcrumbs && "border-t border-line",
                sidebar && "border-l border-line md:rounded-tl-[24px]",
              )}
            >
              {children}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Scrollable page content inside the shell: padded, width-tiered, or edge-to-edge for maps. */
export function ShellContent({
  layout = "default",
  header,
  footer,
  children,
}: {
  /** `full` = edge-to-edge (maps), `wide` = full width, `default` = readable column. */
  layout?: "default" | "wide" | "full";
  header?: React.ReactNode;
  footer?: React.ReactNode;
  children: React.ReactNode;
}) {
  if (layout === "full") {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Maps own the whole content area; the breadcrumbs already name the page. */}
        <div className="relative min-h-0 flex-1">{children}</div>
      </div>
    );
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className={cn("flex w-full flex-1 flex-col gap-6 px-4 py-6 sm:px-6", layout === "default" && "mx-auto max-w-7xl")}>
        {header}
        {children}
      </div>
      {footer}
    </div>
  );
}

export function Sidebar({
  collapsed = false,
  header,
  footer,
  children,
  className,
}: {
  collapsed?: boolean;
  /** Chrome row, identity switcher, ⌘K pill. */
  header?: React.ReactNode;
  footer?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <aside
      data-collapsed={collapsed || undefined}
      className={cn(
        "group/sidebar hidden shrink-0 flex-col bg-panel transition-[width] duration-200 ease-out md:flex",
        collapsed ? "w-[60px]" : "w-60",
        className,
      )}
    >
      {header && <div className="flex flex-col gap-2 px-2.5 pb-2 pt-3">{header}</div>}
      <nav className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-2.5 pb-3">{children}</nav>
      {footer && <div className="border-t border-line-soft px-2.5 py-2">{footer}</div>}
    </aside>
  );
}

export function SidebarGroup({
  label,
  className,
  children,
}: {
  label?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("flex flex-col gap-0.5", className)}>
      {label && (
        <div className="px-2 pb-1 pt-1.5 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-ink-3 group-data-[collapsed]/sidebar:sr-only">
          {label}
        </div>
      )}
      {children}
    </div>
  );
}

/**
 * A sidebar link: icon, label, optional mono count. Pass the router link as the child with
 * `asChild`; the active state keys off `aria-current="page"`, `data-status="active"` or `.active`,
 * which router links set.
 */
export const SidebarItem = React.forwardRef<
  HTMLAnchorElement,
  React.AnchorHTMLAttributes<HTMLAnchorElement> & {
    asChild?: boolean;
    icon: LucideIcon;
    label: React.ReactNode;
    count?: React.ReactNode;
  }
>(({ asChild, icon: Icon, label, count, className, children, ...props }, ref) => {
  const Comp = asChild ? Slot : "a";
  return (
    <Comp
      ref={ref}
      title={typeof label === "string" ? label : undefined}
      className={cn(
        "group/item flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-[13px] text-ink-2 transition-colors hover:bg-panel-2 hover:text-ink",
        "aria-[current=page]:bg-panel-2 aria-[current=page]:text-ink data-[status=active]:bg-panel-2 data-[status=active]:text-ink [&.active]:bg-panel-2 [&.active]:text-ink",
        "group-data-[collapsed]/sidebar:justify-center group-data-[collapsed]/sidebar:px-0",
        className,
      )}
      {...props}
    >
      {asChild ? (
        React.cloneElement(
          children as React.ReactElement<{ children?: React.ReactNode }>,
          undefined,
          <ItemInner icon={Icon} label={label} count={count} />,
        )
      ) : (
        <ItemInner icon={Icon} label={label} count={count} />
      )}
    </Comp>
  );
});
SidebarItem.displayName = "SidebarItem";

function ItemInner({ icon: Icon, label, count }: { icon: LucideIcon; label: React.ReactNode; count?: React.ReactNode }) {
  return (
    <>
      <Icon className="size-[17px] shrink-0 text-ink-3 group-hover/item:text-ink-2 group-aria-[current=page]/item:text-brand-ink group-data-[status=active]/item:text-brand-ink group-[.active]/item:text-brand-ink" />
      <span className="min-w-0 flex-1 truncate group-data-[collapsed]/sidebar:sr-only">{label}</span>
      {count != null && (
        <span className="font-mono text-[11px] text-ink-3 group-data-[collapsed]/sidebar:hidden">{count}</span>
      )}
    </>
  );
}

export type Crumb = {
  label: React.ReactNode;
  icon?: LucideIcon;
  /** Render the crumb as a link: receives the crumb's content. Omit for plain text. */
  link?: (content: React.ReactNode) => React.ReactNode;
};

/** Muted parents, bright current crumb with its icon (DESIGN.md "Wayfinding"). */
export function Breadcrumbs({ items, className }: { items: readonly Crumb[]; className?: string }) {
  return (
    <nav aria-label="Breadcrumb" className={cn("flex min-w-0 items-center gap-2 text-[12.5px] text-ink-3", className)}>
      {items.map((c, i) => {
        const last = i === items.length - 1;
        const Icon = c.icon;
        const content = (
          <span className={cn("flex min-w-0 items-center gap-1.5", last && "text-ink")}>
            {last && Icon && <Icon className="size-3.5 shrink-0" />}
            <span className="truncate">{c.label}</span>
          </span>
        );
        return (
          <React.Fragment key={i}>
            {i > 0 && <ChevronRight className="size-3 shrink-0" aria-hidden="true" />}
            {!last && c.link ? <span className="hover:text-ink-2">{c.link(content)}</span> : content}
          </React.Fragment>
        );
      })}
    </nav>
  );
}

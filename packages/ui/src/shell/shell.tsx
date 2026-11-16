import * as React from "react";
import {Slot} from "@radix-ui/react-slot";
import {ChevronRight, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import {Tooltip, TooltipContent, TooltipTrigger} from "../components/tooltip";
import {DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuTrigger} from "../components/dropdown-menu";

/**
 * The console shell (DESIGN.md "The shell"): full-height, flush to the viewport, holding the
 * collapsible sidebar and the main area. The main area is a breadcrumb row over the page's content
 * panel, inset and rounded on all four corners.
 *
 * The outer edge of the app *is* the viewport — no gutter, no CSS rounding of its own and no frame
 * shadow (#402). On the desktop app that viewport sits inside the OS's own title bar and frame (#796),
 * and any rounding of the window's corners is the OS's, so there is deliberately no `border-radius`
 * here to match. The page's own 16px side gutters come from
 * {@link ShellContent}'s padding, and the content panel is held off the window edge by its own
 * `mx-2 mb-2` — with the gutter gone there is nothing else to do it.
 *
 * `--ground` is gone from the outer element with it: the frame is the outer element's only child and
 * stretches over the whole `h-dvh` box, so a ground fill behind it could never paint.
 */
export function Shell({
  sidebar,
  breadcrumbs,
  leading,
  children,
  className,
}: {
  /** A `<Sidebar>` (hidden below `md`; pass a menu button as `leading` for phones). */
  sidebar?: React.ReactNode;
  breadcrumbs?: React.ReactNode;
  /** Sits before the breadcrumbs (e.g. the phone menu button). */
  leading?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-dvh text-ink", className)}>
      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden bg-panel">
        {sidebar}
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-11 shrink-0 items-center gap-2 px-3 md:px-5">
            {leading}
            {breadcrumbs}
          </div>
          <div className="relative mx-2 mb-2 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-line md:ml-0">
            {children}
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
  children,
}: {
  /** `full` = edge-to-edge (maps), `wide` = full width, `default` = readable column. */
  layout?: "default" | "wide" | "full";
  header?: React.ReactNode;
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
    </div>
  );
}

/** `path` is the current route's pathname, so an open group flyout closes on any navigation. */
const SidebarContext = React.createContext<{ collapsed: boolean; path?: string }>({ collapsed: false });

/**
 * In a collapsed sidebar, labels are hidden — wrap an icon-only control so hovering or focusing it
 * shows its name to the right. Renders the child untouched when the sidebar is expanded.
 */
export function SidebarTooltip({ label, children }: { label: React.ReactNode; children: React.ReactElement }) {
  const { collapsed } = React.useContext(SidebarContext);
  if (!collapsed) return children;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="right" sideOffset={10}>
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

export function Sidebar({
  collapsed = false,
  path,
  header,
  footer,
  children,
  className,
}: {
  collapsed?: boolean;
  /** The current route's pathname: a change closes any open group flyout. */
  path?: string;
  /** Chrome row, identity switcher, ⌘K pill. */
  header?: React.ReactNode;
  footer?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <SidebarContext.Provider value={{ collapsed, path }}>
    <aside
      data-collapsed={collapsed || undefined}
      className={cn(
        "group/sidebar hidden shrink-0 flex-col bg-panel transition-[width] duration-200 ease-out md:flex",
        collapsed ? "w-[60px]" : "w-60",
        className,
      )}
    >
      {header && <div className="flex flex-col gap-2 px-2.5 pb-2 pt-3">{header}</div>}
      {/* `relative` contains the collapsed labels (`sr-only` is absolute) so they can't lengthen the document. */}
      <nav className="relative flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-2.5 pb-3">{children}</nav>
      {footer && <div className="border-t border-line-soft px-2.5 py-2">{footer}</div>}
    </aside>
    </SidebarContext.Provider>
  );
}

/**
 * A labelled run of sidebar items. In a collapsed sidebar, a group with a `label` and an `icon` is one
 * icon that opens its items in a flyout to the right (#823); `active` marks that icon when the current
 * route is inside the group. Selecting an item closes the flyout, except a child carrying
 * `data-keep-open` (a confirm-in-place button). Expanded, or outside a `Sidebar`, `icon` and `active`
 * change nothing.
 */
export function SidebarGroup({
  label,
  icon,
  active,
  className,
  children,
}: {
  label?: React.ReactNode;
  icon?: LucideIcon;
  active?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  const { collapsed } = React.useContext(SidebarContext);
  if (collapsed && label && icon) {
    return (
      <SidebarFlyout label={label} icon={icon} active={active} className={className}>
        {children}
      </SidebarFlyout>
    );
  }
  return (
    <div className={cn("flex flex-col gap-0.5", className)}>
      {label && (
        <div className={cn(GROUP_LABEL, "group-data-[collapsed]/sidebar:sr-only")}>
          {label}
        </div>
      )}
      {children}
    </div>
  );
}

const GROUP_LABEL = "px-2 pb-1 pt-1.5 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-ink-3";

/** A collapsed group: one icon that opens the group's items, with their labels, to its right. */
function SidebarFlyout({
  label,
  icon: Icon,
  active,
  className,
  children,
}: {
  label: React.ReactNode;
  icon: LucideIcon;
  active?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  const { path } = React.useContext(SidebarContext);
  const [open, setOpen] = React.useState(false);
  // Adjusting state while rendering (not in an effect) closes it in the same render the route changes.
  const [openedPath, setOpenedPath] = React.useState(path);
  if (path !== openedPath) {
    setOpenedPath(path);
    setOpen(false);
  }
  return (
    <div className={cn("flex flex-col", className)}>
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <SidebarTooltip label={label}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              data-active={active || undefined}
              className="group/item flex w-full items-center justify-center rounded-sm py-2 text-ink-2 outline-none transition-colors hover:bg-panel-2 hover:text-ink focus-visible:ring-2 focus-visible:ring-ring data-[active]:bg-panel-2 data-[active]:text-ink aria-[expanded=true]:bg-panel-2"
            >
              <Icon className="size-[17px] shrink-0 text-ink-3 group-hover/item:text-ink-2 group-data-[active]/item:text-brand-ink" />
              <span className="sr-only">{label}</span>
            </button>
          </DropdownMenuTrigger>
        </SidebarTooltip>
        <DropdownMenuContent side="right" align="start" sideOffset={10} className="rounded-md bg-panel p-1.5">
          <DropdownMenuLabel className={GROUP_LABEL}>{label}</DropdownMenuLabel>
          {/* Inside the flyout the labels show, so items drop their collapsed-mode tooltips. */}
          <SidebarContext.Provider value={{ collapsed: false, path }}>
            {React.Children.map(children, (child) => {
              if (!React.isValidElement(child)) return child;
              const keepOpen = (child.props as Record<string, unknown>)["data-keep-open"] != null;
              return (
                <DropdownMenuPrimitive.Item
                  asChild
                  className="outline-none data-[highlighted]:bg-panel-2 data-[highlighted]:text-ink"
                  onSelect={keepOpen ? (e) => e.preventDefault() : undefined}
                >
                  {child}
                </DropdownMenuPrimitive.Item>
              );
            })}
          </SidebarContext.Provider>
        </DropdownMenuContent>
      </DropdownMenu>
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
    /** Extra classes for the icon — e.g. a hover treatment DESIGN.md names as an exception. */
    iconClassName?: string;
  }
>(({ asChild, icon: Icon, label, count, iconClassName, className, children, ...props }, ref) => {
  const Comp = asChild ? Slot : "a";
  return (
    <SidebarTooltip label={label}>
    <Comp
      ref={ref}
      className={cn(
        "group/item flex w-full items-center gap-2.5 rounded-sm px-2.5 py-2 text-left text-[13px] text-ink-2 transition-colors hover:bg-panel-2 hover:text-ink",
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
          <ItemInner icon={Icon} label={label} count={count} iconClassName={iconClassName} />,
        )
      ) : (
        <ItemInner icon={Icon} label={label} count={count} iconClassName={iconClassName} />
      )}
    </Comp>
    </SidebarTooltip>
  );
});
SidebarItem.displayName = "SidebarItem";

function ItemInner({
  icon: Icon,
  label,
  count,
  iconClassName,
}: {
  icon: LucideIcon;
  label: React.ReactNode;
  count?: React.ReactNode;
  iconClassName?: string;
}) {
  return (
    <>
      <Icon
        className={cn(
          "size-[17px] shrink-0 text-ink-3 group-hover/item:text-ink-2 group-aria-[current=page]/item:text-brand-ink group-data-[status=active]/item:text-brand-ink group-[.active]/item:text-brand-ink",
          iconClassName,
        )}
      />
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

/** Muted parents, bright current crumb with its icon (DESIGN.md "The shell"). */
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

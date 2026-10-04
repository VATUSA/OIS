import * as React from "react";
import {Slot} from "@radix-ui/react-slot";
import {ChevronRight, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {Tooltip, TooltipContent, TooltipTrigger} from "../components/tooltip";

/**
 * The console shell (DESIGN.md "The shell"): full-height, flush to the viewport, holding the
 * collapsible sidebar and the main area. The main area is a breadcrumb row over the page's content
 * panel, inset and rounded on all four corners.
 *
 * The outer edge of the app *is* the window's bounding box — no gutter, no CSS rounding of its own and
 * no frame shadow (#402). On the desktop app the native title bar is hidden, so a gutter here would
 * have read as a second chrome bar under it.
 *
 * The *window* does have rounded corners, put there by the OS rather than by this component (#419):
 * macOS rounds a decorated window itself and Windows 11 is asked to via DWM, and the compositor clips
 * the webview to that shape. So there is deliberately no `border-radius` here to match — adding one
 * would double the curve inside the OS's. The page's own 16px side gutters come from
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
  topBarProps,
  children,
  className,
}: {
  /** A `<Sidebar>` (hidden below `md`; pass a menu button as `leading` for phones). */
  sidebar?: React.ReactNode;
  breadcrumbs?: React.ReactNode;
  /** Sits before the breadcrumbs (e.g. the phone menu button). */
  leading?: React.ReactNode;
  /**
   * Spread onto the top bar. Kept generic so this package stays free of platform assumptions: the
   * desktop app passes a Tauri drag region through it (#402), and the web build passes nothing.
   *
   * `className` is excluded on purpose, and the props are spread *before* it, so a caller cannot
   * replace the row's height and padding — by the type or by accident.
   *
   * `onDoubleClick` is excluded for a sharper reason: when this row is a Tauri drag region, Tauri's
   * own injected `drag.js` already toggles maximize on a double-click inside it. A handler here fires
   * *as well*, so one double-click toggled twice — dead on Windows and Linux, unrestorable on macOS
   * (#402 review). React's synthetic events also bubble, where Tauri's drag matching is self-only, so
   * the same handler fired for every button and breadcrumb in the row. Excluding it makes putting one
   * back a type error rather than a defect a reviewer has to find twice.
   */
  topBarProps?: Omit<React.HTMLAttributes<HTMLDivElement>, "className" | "onDoubleClick"> & {
    [attr: `data-${string}`]: unknown;
  };
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-dvh text-ink", className)}>
      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden bg-panel">
        {sidebar}
        <div className="flex min-w-0 flex-1 flex-col">
          {/* Spread first: the row's own layout classes must win over anything a caller passes. */}
          <div {...topBarProps} className="flex h-11 shrink-0 items-center gap-2 px-3 md:px-5">
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

const SidebarContext = React.createContext({ collapsed: false });

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
    <SidebarContext.Provider value={{ collapsed }}>
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

import {useEffect} from "react";
import {Link, useNavigate, useRouterState} from "@tanstack/react-router";
import {Breadcrumbs, type Crumb, PageHeader, Shell, ShellContent, useLocalStorage} from "@ois/ui";
import {Home} from "lucide-react";

import {useMe} from "@/lib/auth";
import {areaForPath, groupForPath, itemForPath, visibleGroups} from "@/lib/nav";

import {AppSidebar, MobileNavButton} from "./app-sidebar";
import {CommandSearch} from "./command-search";
import {UpdateBanner} from "@/components/update-banner";
import {OpenInWindowButton} from "@/components/shell/open-in-window";
import {useDragRegionProps} from "@/components/shell/window-controls";
import {PageMetaProvider, usePageHeaderOverride, usePageTitle, useRouteMeta, useView} from "./page-meta";
import {recordVisit} from "./recent-pages";

/** The signed-in user's own pages (the sidebar's User group). */
const USER_PAGES = new Set(["/profile", "/settings", "/api-keys"]);

function useCrumbs(title: string | undefined): Crumb[] {
  const { data: me } = useMe();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const area = areaForPath(pathname);
  if (pathname === "/") return [{ label: "Home", icon: Home }];
  if (USER_PAGES.has(pathname)) return [{ label: "User" }, ...(title ? [{ label: title }] : [])];
  if (!area) return title ? [{ label: "Home", icon: Home, link: (c) => <Link to="/">{c}</Link> }, { label: title }] : [];
  const first = visibleGroups(me, area)[0]?.items[0];
  const hit = itemForPath(pathname);
  const crumbs: Crumb[] = [
    { label: area.label, link: first ? (c) => <Link to={area.id === "admin" ? "/admin" : first.to}>{c}</Link> : undefined },
  ];
  const groupLabel = hit?.group.label ?? groupForPath(pathname)?.label;
  if (groupLabel && groupLabel !== area.label) crumbs.push({ label: groupLabel });
  if (hit) crumbs.push({ label: hit.item.label, icon: hit.item.icon, link: (c) => <Link to={hit.item.to}>{c}</Link> });
  // The page title adds a crumb only below a nav item (an event, a flight), not on the item's own page.
  if (title && (!hit || pathname !== hit.item.to)) crumbs.push({ label: title });
  return crumbs;
}

function Header() {
  const meta = useRouteMeta();
  const override = usePageHeaderOverride();
  const view = useView();
  const navigate = useNavigate();
  const title = override.title ?? meta.title;
  if (!title) return null;
  return (
    <PageHeader
      title={title}
      icon={meta.icon}
      count={override.count}
      subtitle={override.subtitle ?? meta.subtitle}
      actions={override.actions}
      views={override.views === null ? undefined : meta.views}
      view={view}
      onViewChange={(v) =>
        void navigate({ to: ".", search: (prev: Record<string, unknown>) => ({ ...prev, view: v }) } as never)
      }
    />
  );
}

function Frame({ children }: { children: React.ReactNode }) {
  const meta = useRouteMeta();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const [collapsed, setCollapsed] = useLocalStorage("ois.sidebar.collapsed", false);
  // Shared with whatever else names this page — favoriting it, the recents list — so a stored label
  // cannot drift from the title on screen (VATUSA/OIS#312).
  const title = usePageTitle();
  const crumbs = useCrumbs(title);
  const dragRegion = useDragRegionProps();

  useEffect(() => {
    if (title) recordVisit({ path: pathname, label: title });
  }, [pathname, title]);

  return (
    <Shell
      sidebar={<AppSidebar collapsed={collapsed} onToggle={() => setCollapsed(!collapsed)} />}
      leading={<MobileNavButton />}
      // The window's own buttons are not here: they belong at the window's top-left, which is the
      // sidebar's chrome row (#419) — on macOS the OS draws them there itself and the page cannot move
      // them. This row is still what *moves* the window, on every platform: macOS's `Overlay` title bar
      // is transparent and sits over the content, so it needs the drag region as much as an
      // undecorated Windows window does.
      // Double-click-to-maximize is Tauri's own, not ours: `drag.js` already does it for a drag
      // region and gets the macOS/Windows difference right, and a handler of ours on top toggled
      // twice. Nothing is passed on the web build or in a route window — see useDragRegionProps.
      topBarProps={dragRegion}
      breadcrumbs={
        <>
          {crumbs.length > 0 && <Breadcrumbs items={crumbs} />}
          <OpenInWindowButton />
        </>
      }
    >
      <ShellContent layout={meta.layout} header={<Header />}>
        {children}
      </ShellContent>
    </Shell>
  );
}

/** The app frame for every non-embedded page. */
export function AppShell({ children }: { children: React.ReactNode }) {
  return (
    <PageMetaProvider>
      {/* Inside the frame, not above it. The banner is an in-flow strip, and anything in flow above
          the shell pushes the sidebar's chrome row down while the OS keeps the macOS traffic lights
          at the fixed window coordinates `trafficLightPosition` gives them (#419 review) — a staged
          update put the real lights on top of the banner and left an empty gap in the row. */}
      <Frame>
        <UpdateBanner />
        {children}
      </Frame>
      <CommandSearch />
    </PageMetaProvider>
  );
}

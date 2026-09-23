import {useEffect} from "react";
import {Link, useNavigate, useRouterState} from "@tanstack/react-router";
import {Breadcrumbs, type Crumb, PageHeader, Shell, ShellContent, useLocalStorage} from "@ois/ui";
import {Home} from "lucide-react";

import {useMe} from "@/lib/auth";
import {areaForPath, groupForPath, itemForPath, visibleGroups} from "@/lib/nav";

import {AppSidebar, MobileNavButton} from "./app-sidebar";
import {CommandSearch} from "./command-search";
import {UpdateBanner} from "@/components/update-banner";
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

  useEffect(() => {
    if (title) recordVisit({ path: pathname, label: title });
  }, [pathname, title]);

  return (
    <Shell
      sidebar={<AppSidebar collapsed={collapsed} onToggle={() => setCollapsed(!collapsed)} />}
      leading={<MobileNavButton />}
      breadcrumbs={crumbs.length > 0 ? <Breadcrumbs items={crumbs} /> : undefined}
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
      <UpdateBanner />
      <Frame>{children}</Frame>
      <CommandSearch />
    </PageMetaProvider>
  );
}

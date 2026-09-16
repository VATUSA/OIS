import {useEffect} from "react";
import {Link, useNavigate, useRouterState} from "@tanstack/react-router";
import {Breadcrumbs, type Crumb, PageHeader, Shell, ShellContent, useLocalStorage} from "@ois/ui";
import {Home} from "lucide-react";

import {Footer} from "@/components/footer";
import {useMe} from "@/lib/auth";
import {areaForPath, itemForPath, visibleGroups} from "@/lib/nav";

import {AreaSidebar} from "./area-sidebar";
import {CommandSearch} from "./command-search";
import {PageMetaProvider, usePageHeaderOverride, useRouteMeta, useView} from "./page-meta";
import {recordVisit} from "./recent-pages";
import {TopNav} from "./top-nav";

function useCrumbs(title: string | undefined): Crumb[] {
  const { data: me } = useMe();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const area = areaForPath(pathname);
  if (!area) return title ? [{ label: "Home", icon: Home, link: (c) => <Link to="/">{c}</Link> }, { label: title }] : [];
  const first = visibleGroups(me, area)[0]?.items[0];
  const hit = itemForPath(pathname);
  const crumbs: Crumb[] = [
    { label: area.label, link: first ? (c) => <Link to={area.id === "admin" ? "/admin" : first.to}>{c}</Link> : undefined },
  ];
  if (hit?.group.label && hit.group.label !== area.label) crumbs.push({ label: hit.group.label });
  if (hit) crumbs.push({ label: hit.item.label, icon: hit.item.icon, link: (c) => <Link to={hit.item.to}>{c}</Link> });
  if (title && title !== hit?.item.label) crumbs.push({ label: title });
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
      views={meta.views}
      view={view}
      onViewChange={(v) =>
        void navigate({ to: ".", search: (prev: Record<string, unknown>) => ({ ...prev, view: v }) } as never)
      }
    />
  );
}

function Frame({ children }: { children: React.ReactNode }) {
  const meta = useRouteMeta();
  const override = usePageHeaderOverride();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { data: me } = useMe();
  const [collapsed, setCollapsed] = useLocalStorage("ois.sidebar.collapsed", false);
  const area = areaForPath(pathname);
  const showSidebar = area != null && visibleGroups(me, area).length > 0;
  const title = override.title ?? meta.title ?? itemForPath(pathname)?.item.label;
  const crumbs = useCrumbs(title);

  useEffect(() => {
    if (title) recordVisit({ path: pathname, label: title });
  }, [pathname, title]);

  return (
    <Shell
      topBar={<TopNav />}
      sidebar={showSidebar && area ? <AreaSidebar area={area} collapsed={collapsed} onToggle={() => setCollapsed(!collapsed)} /> : undefined}
      breadcrumbs={crumbs.length > 0 ? <Breadcrumbs items={crumbs} /> : undefined}
    >
      <ShellContent layout={meta.layout} header={<Header />} footer={<Footer />}>
        {children}
      </ShellContent>
    </Shell>
  );
}

/** The app frame for every non-embedded page. */
export function AppShell({ children }: { children: React.ReactNode }) {
  return (
    <PageMetaProvider>
      <Frame>{children}</Frame>
      <CommandSearch />
    </PageMetaProvider>
  );
}

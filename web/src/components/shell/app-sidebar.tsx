import {useState} from "react";
import {Link, useRouter} from "@tanstack/react-router";
import {
  Avatar,
  AvatarFallback,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  Modal,
  Sidebar,
  SidebarGroup,
  SidebarItem,
  ThemeToggle,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ois/ui";
import {BookOpen, ChevronDown, ChevronLeft, ChevronRight, History, Home, LogIn, Menu, PanelLeft, Search} from "lucide-react";

import {ZuluClock} from "@/components/zulu-clock";
import {DOCS_URL} from "@/lib/api";
import {login, useMe} from "@/lib/auth";
import {ADMIN_HOME, AREAS, visibleGroups} from "@/lib/nav";

import {openCommandSearch} from "./command-search";
import {useRecentPages} from "./recent-pages";
import {initials, UserMenuItems} from "./user-menu";

function ChromeButton({ label, onClick, children }: { label: string; onClick?: () => void; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button size="icon" variant="ghost" aria-label={label} className="size-7 text-ink-3" onClick={onClick}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/** Back · forward · recent pages, and the collapse toggle (desktop-ready; no traffic lights on web). */
function ChromeRow({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  const router = useRouter();
  const recent = useRecentPages();
  if (collapsed) {
    return (
      <div className="flex justify-center">
        <ChromeButton label="Expand sidebar" onClick={onToggle}>
          <PanelLeft />
        </ChromeButton>
      </div>
    );
  }
  return (
    <div className="flex items-center gap-0.5">
      <ChromeButton label="Back" onClick={() => router.history.back()}>
        <ChevronLeft />
      </ChromeButton>
      <ChromeButton label="Forward" onClick={() => router.history.forward()}>
        <ChevronRight />
      </ChromeButton>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="icon" variant="ghost" aria-label="Recent pages" className="size-7 text-ink-3">
            <History />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuLabel>Recent</DropdownMenuLabel>
          {recent.length === 0 ? (
            <DropdownMenuItem disabled>Nothing yet</DropdownMenuItem>
          ) : (
            recent.map((p) => (
              <DropdownMenuItem key={p.path} asChild>
                <Link to={p.path}>{p.label}</Link>
              </DropdownMenuItem>
            ))
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <span className="flex-1" />
      <ChromeButton label="Collapse sidebar" onClick={onToggle}>
        <PanelLeft />
      </ChromeButton>
    </div>
  );
}

/** The workspace switcher, as the signed-in identity: avatar, name, mono CID · rating. */
function IdentitySwitcher({ collapsed }: { collapsed: boolean }) {
  const { data: me } = useMe();
  if (!me) {
    return collapsed ? (
      <Button size="icon" aria-label="Sign in with VATSIM" onClick={login} className="mx-auto">
        <LogIn />
      </Button>
    ) : (
      <Button size="sm" onClick={login}>
        Sign in with VATSIM
      </Button>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-2.5 rounded-sm border-b border-line-soft px-1.5 pb-3 pt-1 text-left outline-none hover:bg-panel-2 focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Avatar className="size-[30px] rounded-sm">
            <AvatarFallback className="rounded-sm">{initials(me.display_name)}</AvatarFallback>
          </Avatar>
          {!collapsed && (
            <>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13.5px] font-semibold text-ink">{me.display_name}</span>
                <span className="block truncate font-mono text-[10.5px] text-ink-3">
                  CID {me.cid}
                  {me.rating ? ` · ${me.rating}` : ""}
                </span>
              </span>
              <ChevronDown className="size-3.5 text-ink-3" />
            </>
          )}
        </button>
      </DropdownMenuTrigger>
      {/* Expanded, the menu matches the sidebar's width; collapsed, it keeps its natural width. */}
      <DropdownMenuContent align="start" className={collapsed ? undefined : "w-[var(--radix-dropdown-menu-trigger-width)]"}>
        <UserMenuItems me={me} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SearchPill({ collapsed }: { collapsed: boolean }) {
  if (collapsed) {
    return (
      <div className="flex justify-center">
        <ChromeButton label="Search (⌘K)" onClick={openCommandSearch}>
          <Search />
        </ChromeButton>
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={openCommandSearch}
      className="flex items-center gap-2 rounded-full border border-line bg-panel-2 px-3 py-1.5 text-[12.5px] text-ink-3 transition-colors hover:text-ink-2"
    >
      <Search className="size-3.5" />
      Search
      <kbd className="ml-auto rounded-[4px] border border-line px-1.5 font-mono text-[10px]">⌘K</kbd>
    </button>
  );
}

/**
 * Every section the user can use, grouped: Home, then Advisories, Operations, Planning, Historical and
 * Admin (its landing first). Shared by the desktop sidebar and the phone drawer.
 */
function NavGroups() {
  const { data: me } = useMe();
  const groups = AREAS.flatMap((area) =>
    visibleGroups(me, area).map((g) => ({
      key: `${area.id}-${g.label ?? ""}`,
      label: g.label ?? area.label,
      items: area.id === "admin" && g.label === "Admin" ? [ADMIN_HOME, ...g.items] : g.items,
    })),
  );
  return (
    <>
      <SidebarGroup>
        <SidebarItem asChild icon={Home} label="Home">
          <Link to="/" activeOptions={{ exact: true }} />
        </SidebarItem>
      </SidebarGroup>
      {groups.map((g) => (
        <SidebarGroup key={g.key} label={g.label}>
          {g.items.map((item) => (
            <SidebarItem key={item.to} asChild icon={item.icon} label={item.label}>
              <Link to={item.to} activeOptions={{ exact: item.exact }} />
            </SidebarItem>
          ))}
        </SidebarGroup>
      ))}
    </>
  );
}

function SidebarFooter({ collapsed }: { collapsed: boolean }) {
  return (
    <div className={collapsed ? "flex flex-col items-center gap-1" : "flex items-center gap-1"}>
      {DOCS_URL && (
        <ChromeButton label="Docs" onClick={() => window.open(DOCS_URL, "_blank", "noreferrer")}>
          <BookOpen />
        </ChromeButton>
      )}
      <ThemeToggle />
      {!collapsed && <ZuluClock className="ml-auto rounded-full border border-line bg-panel-2 px-2.5 py-1 font-mono text-xs text-ink-2" />}
    </div>
  );
}

/** The one global sidebar: chrome row, identity, ⌘K, every permitted section, then Docs/theme/clock. */
export function AppSidebar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  return (
    <Sidebar
      collapsed={collapsed}
      header={
        <>
          <ChromeRow collapsed={collapsed} onToggle={onToggle} />
          <IdentitySwitcher collapsed={collapsed} />
          <SearchPill collapsed={collapsed} />
        </>
      }
      footer={<SidebarFooter collapsed={collapsed} />}
    >
      <NavGroups />
    </Sidebar>
  );
}

/** Phones: the sidebar's contents in a left drawer, opened from the breadcrumb row. */
export function MobileNavButton() {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  return (
    <>
      <Button size="icon" variant="ghost" aria-label="Menu" className="size-8 md:hidden" onClick={() => setOpen(true)}>
        <Menu className="size-5" />
      </Button>
      <Modal open={open} onClose={close} placement="left" title="OIS">
        <nav className="flex flex-col gap-3" onClick={(e) => (e.target as HTMLElement).closest("a") && close()}>
          <IdentitySwitcher collapsed={false} />
          <SearchPill collapsed={false} />
          <NavGroups />
          <SidebarFooter collapsed={false} />
        </nav>
      </Modal>
    </>
  );
}

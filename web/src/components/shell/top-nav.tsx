import {useState} from "react";
import {Link, useRouterState} from "@tanstack/react-router";
import {
  Avatar,
  AvatarFallback,
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  Modal,
  SidebarGroup,
  SidebarItem,
  ThemeToggle,
} from "@ois/ui";
import {BookOpen, ChevronDown, Home, Menu, Search} from "lucide-react";

import vatusaLogo from "@/assets/vatusa-logo.png";
import {ZuluClock} from "@/components/zulu-clock";
import {DOCS_URL} from "@/lib/api";
import {login, useMe} from "@/lib/auth";
import {areaById, type NavArea, visibleGroups} from "@/lib/nav";

import {openCommandSearch} from "./command-search";
import {initials, UserMenuItems} from "./user-menu";

const linkClass =
  "rounded-full px-3 py-1.5 text-sm text-ink-2 transition-colors hover:bg-panel-2 hover:text-ink data-[status=active]:text-ink";

/** A top-nav area dropdown: its visible links, highlighted while inside the area. */
function AreaMenu({ area }: { area: NavArea }) {
  const { data: me } = useMe();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const groups = visibleGroups(me, area);
  if (groups.length === 0) return null;
  const active = area.prefixes.some((p) => pathname === p || pathname.startsWith(`${p}/`));
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className={cn(linkClass, "flex items-center gap-1 outline-none", active && "text-ink")}>
        {area.label}
        <ChevronDown className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {groups.map((g, gi) => (
          <div key={gi}>
            {g.label && <DropdownMenuLabel>{g.label}</DropdownMenuLabel>}
            {g.items.map((item) => (
              <DropdownMenuItem key={item.to} asChild>
                <Link to={item.to}>
                  <item.icon />
                  {item.label}
                </Link>
              </DropdownMenuItem>
            ))}
          </div>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AvatarMenu() {
  const { data: me, isLoading } = useMe();
  if (isLoading) return <div className="size-8 animate-pulse rounded-full bg-panel-2" />;
  if (!me) {
    return (
      <Button size="sm" onClick={login}>
        Sign in with VATSIM
      </Button>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button aria-label="Account" className="flex items-center gap-1 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <Avatar className="size-8">
            <AvatarFallback>{initials(me.display_name)}</AvatarFallback>
          </Avatar>
          <ChevronDown className="size-3.5 text-ink-3" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <UserMenuItems me={me} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Every area's links in a left drawer — the phone-width nav. */
function MobileNav() {
  const { data: me } = useMe();
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  return (
    <>
      <Button size="icon" variant="ghost" aria-label="Menu" className="md:hidden" onClick={() => setOpen(true)}>
        <Menu className="size-5" />
      </Button>
      <Modal open={open} onClose={close} placement="left" title="OIS">
        <nav className="flex flex-col gap-4" onClick={(e) => (e.target as HTMLElement).closest("a") && close()}>
          <SidebarGroup>
            <SidebarItem asChild icon={Home} label="Home">
              <Link to="/" activeOptions={{ exact: true }} />
            </SidebarItem>
          </SidebarGroup>
          {(["advisories", "operations", "admin"] as const).map((id) => {
            const area = areaById(id);
            return visibleGroups(me, area).map((g, gi) => (
              <SidebarGroup key={`${id}-${gi}`} label={g.label ? `${area.label} · ${g.label}` : area.label}>
                {g.items.map((item) => (
                  <SidebarItem key={item.to} asChild icon={item.icon} label={item.label}>
                    <Link to={item.to} activeOptions={{ exact: item.exact }} />
                  </SidebarItem>
                ))}
              </SidebarGroup>
            ));
          })}
          {DOCS_URL && (
            <SidebarGroup>
              <SidebarItem icon={BookOpen} label="Docs" href={DOCS_URL} target="_blank" rel="noreferrer" />
            </SidebarGroup>
          )}
        </nav>
      </Modal>
    </>
  );
}

/** `Home · Advisories ▾ · Operations ▾ · Docs · (avatar ▾)` — Planning/Historical live under Admin. */
export function TopNav() {
  return (
    <header className="flex h-14 shrink-0 items-center gap-2 px-3 sm:px-4">
      <MobileNav />
      <Link to="/" className="mr-2 flex items-center gap-2 font-bold text-ink">
        <img src={vatusaLogo} alt="" className="size-6" />
        <span>OIS</span>
      </Link>
      <nav className="hidden items-center gap-0.5 md:flex">
        <Link to="/" activeOptions={{ exact: true }} className={linkClass}>
          Home
        </Link>
        <AreaMenu area={areaById("advisories")} />
        <AreaMenu area={areaById("operations")} />
        {DOCS_URL && (
          <a href={DOCS_URL} target="_blank" rel="noreferrer" className={linkClass}>
            Docs
          </a>
        )}
      </nav>
      <div className="ml-auto flex items-center gap-1.5">
        <Button size="icon" variant="ghost" aria-label="Search (⌘K)" onClick={openCommandSearch}>
          <Search />
        </Button>
        <ZuluClock className="hidden rounded-full border border-line bg-panel-2 px-2.5 py-1 font-mono text-xs text-ink-2 sm:inline" />
        <ThemeToggle />
        <AvatarMenu />
      </div>
    </header>
  );
}

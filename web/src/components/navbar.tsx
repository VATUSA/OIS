import {Link, useRouterState} from "@tanstack/react-router";
import {
  Avatar,
  AvatarFallback,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  ThemeToggle,
} from "@ois/ui";
import {
  BookOpen,
  CalendarClock,
  ChevronDown,
  FileText,
  Film,
  Gauge,
  KeyRound,
  LayoutDashboard,
  LogOut,
  Megaphone,
  Menu,
  MapPinned,
  Plane,
  PlaneTakeoff,
  Radar,
  Settings as SettingsIcon,
  ShieldCheck,
  Split,
  Timer,
  User as UserIcon,
  TrendingUp,
  Waypoints,
  Wind,
} from "lucide-react";

import {ZuluClock} from "@/components/zulu-clock";
import {DOCS_URL} from "@/lib/api";
import {login, useLogout, useMe} from "@/lib/auth";
import {useFeedStatus} from "@/lib/feed";
import {hasPermission, isAdmin} from "@/lib/permissions";

function initials(name: string) {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]!.toUpperCase())
    .join("");
}

/** A top-nav dropdown group whose trigger highlights while on a matching route. */
function NavGroup({
  label,
  activePrefix,
  children,
}: {
  label: string;
  activePrefix: string;
  children: React.ReactNode;
}) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const active = pathname.startsWith(activePrefix);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={
          "flex items-center gap-1 rounded-md px-3 py-1.5 outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring " +
          (active ? "text-foreground" : "")
        }
      >
        {label}
        <ChevronDown className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">{children}</DropdownMenuContent>
    </DropdownMenu>
  );
}

function UserMenu() {
  const { data: me, isLoading } = useMe();
  const logout = useLogout();

  if (isLoading) {
    return <div className="size-9 animate-pulse rounded-full bg-muted" />;
  }

  if (!me) {
    return <Button onClick={login}>Sign in with VATSIM</Button>;
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <Avatar>
            <AvatarFallback>{initials(me.display_name)}</AvatarFallback>
          </Avatar>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuLabel>
          <div className="flex flex-col">
            <span>{me.display_name}</span>
            <span className="text-xs font-normal text-muted-foreground">
              CID {me.cid}
              {me.rating ? ` · ${me.rating}` : ""}
            </span>
          </div>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/profile">
            <UserIcon />
            Profile
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/settings">
            <SettingsIcon />
            Settings
          </Link>
        </DropdownMenuItem>
        {hasPermission(me, "api_keys.key.create") && (
          <DropdownMenuItem asChild>
            <Link to="/api-keys">
              <KeyRound />
              API keys
            </Link>
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        {isAdmin(me) && (
          <>
            <DropdownMenuItem asChild>
              <Link to="/admin">
                <ShieldCheck />
                Admin
              </Link>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
          </>
        )}
        <DropdownMenuItem
          onSelect={() => logout.mutate()}
          className="text-destructive focus:text-destructive"
        >
          <LogOut />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The full nav collapsed into a hamburger dropdown for small screens. */
function MobileMenu({
  canOps,
  canPrograms,
  canFca,
  canRunway,
  canPlan,
  canStats,
}: {
  canOps: boolean;
  canPrograms: boolean;
  canFca: boolean;
  canRunway: boolean;
  canPlan: boolean;
  canStats: boolean;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label="Menu"
        className="flex items-center rounded-md p-1.5 text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring sm:hidden"
      >
        <Menu className="size-5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-[80vh] w-56 overflow-y-auto">
        <DropdownMenuItem asChild>
          <Link to="/">Home</Link>
        </DropdownMenuItem>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>Advisories</DropdownMenuLabel>
        <DropdownMenuItem asChild>
          <Link to="/advisories">
            <Megaphone />
            TMI board
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/advisories/fcas">
            <Waypoints />
            FCA overview
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/facility-map">
            <Radar />
            Facility map
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/pilot">
            <PlaneTakeoff />
            My flight
          </Link>
        </DropdownMenuItem>

        {canOps && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Operations</DropdownMenuLabel>
            {canPrograms && (
              <DropdownMenuItem asChild>
                <Link to="/ops/airport">
                  <Plane />
                  Airport
                </Link>
              </DropdownMenuItem>
            )}
            <DropdownMenuItem asChild>
              <Link to="/ops/tmu">
                <Gauge />
                TMU
              </Link>
            </DropdownMenuItem>
            {canPrograms && (
              <DropdownMenuItem asChild>
                <Link to="/ops/my">
                  <LayoutDashboard />
                  My dashboard
                </Link>
              </DropdownMenuItem>
            )}
            {canFca && (
              <DropdownMenuItem asChild>
                <Link to="/ops/fca">
                  <Waypoints />
                  FCA flow
                </Link>
              </DropdownMenuItem>
            )}
            {canFca && (
              <DropdownMenuItem asChild>
                <Link to="/ops/idst">
                  <Timer />
                  IDST
                </Link>
              </DropdownMenuItem>
            )}
            {canRunway && (
              <DropdownMenuItem asChild>
                <Link to="/ops/runway">
                  <Split />
                  Runway balancer
                </Link>
              </DropdownMenuItem>
            )}
          </>
        )}

        {canPlan && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Planning</DropdownMenuLabel>
            <DropdownMenuItem asChild>
              <Link to="/planning/events">
                <CalendarClock />
                Events
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/planning/airport-configs">
                <Wind />
                Airport configs
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/planning/facility-documents">
                <FileText />
                Facility documents
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/planning/airport-surface">
                <MapPinned />
                Airport surface data
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/planning/aircraft-profiles">
                <Plane />
                Aircraft profiles
              </Link>
            </DropdownMenuItem>
          </>
        )}

        {canStats && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Historical</DropdownMenuLabel>
            <DropdownMenuItem asChild>
              <Link to="/historical">
                <TrendingUp />
                Network stats
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/historical/dashboard">
                <LayoutDashboard />
                Dashboard replay
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/historical/replay">
                <Film />
                Replay map
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/historical/delays">
                <Timer />
                Delays
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/historical/taxi">
                <PlaneTakeoff />
                Taxi insights
              </Link>
            </DropdownMenuItem>
          </>
        )}

        {DOCS_URL && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem asChild>
              <a href={DOCS_URL} target="_blank" rel="noreferrer">
                <BookOpen />
                Docs
              </a>
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function Navbar() {
  const { data: me } = useMe();
  const linkClass =
    "rounded-md px-3 py-1.5 transition-colors hover:bg-accent hover:text-accent-foreground [&.active]:text-foreground";

  const canPrograms = hasPermission(me, "tmu.program.read");
  const canTmiRead = hasPermission(me, "tmu.tmi.read");
  const canFca = hasPermission(me, "flow.fca.read");
  const canRunway = hasPermission(me, "flow.runway.read");
  const canOps = canPrograms || canTmiRead || canFca || canRunway;
  const canPlan = hasPermission(me, "events.plan.read");
  const canStats = hasPermission(me, "stats.data.read");

  return (
    <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
      <div className="mx-auto flex h-14 w-full max-w-7xl items-center gap-3 px-4 sm:gap-6">
        <MobileMenu
          canOps={canOps}
          canPrograms={canPrograms}
          canFca={canFca}
          canRunway={canRunway}
          canPlan={canPlan}
          canStats={canStats}
        />
        <Link to="/" className="flex items-center gap-2 font-semibold">
          <Radar className="size-5 text-primary" />
          <span>OIS</span>
        </Link>
        <nav className="hidden items-center gap-1 text-sm text-muted-foreground sm:flex">
          <Link to="/" activeOptions={{ exact: true }} className={linkClass}>
            Home
          </Link>

          {/* Public, always visible — pilots view active TMIs + FCAs here. */}
          <NavGroup label="Advisories" activePrefix="/advisories">
            <DropdownMenuItem asChild>
              <Link to="/advisories">
                <Megaphone />
                TMI board
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/advisories/fcas">
                <Waypoints />
                FCA overview
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/facility-map">
                <Radar />
                Facility map
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to="/pilot">
                <PlaneTakeoff />
                My flight
              </Link>
            </DropdownMenuItem>
          </NavGroup>

          {canOps && (
            <NavGroup label="Operations" activePrefix="/ops">
              {canPrograms && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/airport">
                    <Plane />
                    Airport
                  </Link>
                </DropdownMenuItem>
              )}
              {canOps && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/tmu">
                    <Gauge />
                    TMU
                  </Link>
                </DropdownMenuItem>
              )}
              {canPrograms && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/my">
                    <LayoutDashboard />
                    My dashboard
                  </Link>
                </DropdownMenuItem>
              )}
              {canFca && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/fca">
                    <Waypoints />
                    FCA flow
                  </Link>
                </DropdownMenuItem>
              )}
              {canFca && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/idst">
                    <Timer />
                    IDST
                  </Link>
                </DropdownMenuItem>
              )}
              {canRunway && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/runway">
                    <Split />
                    Runway balancer
                  </Link>
                </DropdownMenuItem>
              )}
            </NavGroup>
          )}

          {canPlan && (
            <NavGroup label="Planning" activePrefix="/planning">
              <DropdownMenuItem asChild>
                <Link to="/planning/events">
                  <CalendarClock />
                  Events
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/planning/airport-configs">
                  <Wind />
                  Airport configs
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/planning/facility-documents">
                  <FileText />
                  Facility documents
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/planning/airport-surface">
                  <MapPinned />
                  Airport surface data
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/planning/aircraft-profiles">
                  <Plane />
                  Aircraft profiles
                </Link>
              </DropdownMenuItem>
            </NavGroup>
          )}

          {canStats && (
            <NavGroup label="Historical" activePrefix="/historical">
              <DropdownMenuItem asChild>
                <Link to="/historical">
                  <TrendingUp />
                  Network stats
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/historical/dashboard">
                  <LayoutDashboard />
                  Dashboard replay
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/historical/replay">
                  <Film />
                  Replay map
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/historical/delays">
                  <Timer />
                  Delays
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link to="/historical/taxi">
                  <PlaneTakeoff />
                  Taxi insights
                </Link>
              </DropdownMenuItem>
            </NavGroup>
          )}

          {/* Docs live on their own subdomain (per-environment), wired via DOCS_URL. */}
          {DOCS_URL && (
            <a
              href={DOCS_URL}
              target="_blank"
              rel="noreferrer"
              className={linkClass}
            >
              Docs
            </a>
          )}
        </nav>
        <div className="ml-auto flex items-center gap-2">
          {canPrograms}
          <ZuluClock className="hidden rounded-md border bg-muted/40 px-2 py-1 text-muted-foreground sm:inline" />
          <ThemeToggle />
          <UserMenu />
        </div>
      </div>
    </header>
  );
}

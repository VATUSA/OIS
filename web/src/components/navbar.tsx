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
  CalendarClock,
  ChevronDown,
  Gauge,
  LayoutDashboard,
  LogOut,
  Plane,
  PlaneTakeoff,
  Radar,
  Route,
  ShieldCheck,
  Split,
  Waypoints,
} from "lucide-react";

import {ZuluClock} from "@/components/zulu-clock";
import {login, useLogout, useMe} from "@/lib/auth";
import {useFeedStatus} from "@/lib/feed";
import {hasPermission, isAdmin} from "@/lib/permissions";

function FeedPill() {
  const { data } = useFeedStatus();
  const healthy = !!data?.healthy;
  return (
    <span
      className="hidden items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs text-muted-foreground sm:flex"
      title={
        data
          ? `${data.pilots} pilots · ${data.airports_loaded} airports`
          : "connecting to VATSIM feed"
      }
    >
      <span
        className={
          "size-2 rounded-full " +
          (healthy ? "bg-emerald-500" : "bg-muted-foreground/40")
        }
      />
      {data ? (healthy ? "Feed live" : "Feed down") : "Feed…"}
    </span>
  );
}

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

  return (
    <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
      <div className="mx-auto flex h-14 w-full max-w-7xl items-center gap-6 px-4">
        <Link to="/" className="flex items-center gap-2 font-semibold">
          <Radar className="size-5 text-primary" />
          <span>OIS</span>
        </Link>
        <nav className="hidden items-center gap-1 text-sm text-muted-foreground sm:flex">
          <Link to="/" activeOptions={{ exact: true }} className={linkClass}>
            Home
          </Link>

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
              {canPrograms && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/departures">
                    <PlaneTakeoff />
                    Departures
                  </Link>
                </DropdownMenuItem>
              )}
              {canPrograms && (
                <DropdownMenuItem asChild>
                  <Link to="/ops/taxi">
                    <Route />
                    Taxi
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
            </NavGroup>
          )}
        </nav>
        <div className="ml-auto flex items-center gap-2">
          {canPrograms && <FeedPill />}
          <ZuluClock className="hidden rounded-md border bg-muted/40 px-2 py-1 text-muted-foreground sm:inline" />
          <ThemeToggle />
          <UserMenu />
        </div>
      </div>
    </header>
  );
}

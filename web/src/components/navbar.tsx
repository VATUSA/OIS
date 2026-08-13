import {Link} from "@tanstack/react-router";
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
import {LogOut, Radar} from "lucide-react";

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

  return (
    <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
      <div className="mx-auto flex h-14 w-full max-w-7xl items-center gap-6 px-4">
        <Link to="/" className="flex items-center gap-2 font-semibold">
          <Radar className="size-5 text-primary" />
          <span>OIS</span>
        </Link>
        <nav className="hidden items-center gap-1 text-sm text-muted-foreground sm:flex">
          <Link to="/" activeOptions={{ exact: true }} className={linkClass}>
            Dashboard
          </Link>
          {hasPermission(me, "tmu.program.read") && (
            <Link to="/airport" className={linkClass}>
              Airport
            </Link>
          )}
          {(hasPermission(me, "tmu.program.read") ||
            hasPermission(me, "tmu.tmi.read")) && (
            <Link to="/tmu" className={linkClass}>
              TMU
            </Link>
          )}
          {isAdmin(me) && (
            <Link to="/admin" className={linkClass}>
              Admin
            </Link>
          )}
        </nav>
        <div className="ml-auto flex items-center gap-2">
          {hasPermission(me, "tmu.program.read") && <FeedPill />}
          <ThemeToggle />
          <UserMenu />
        </div>
      </div>
    </header>
  );
}

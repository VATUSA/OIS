import {Link} from "@tanstack/react-router";
import {DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator} from "@ois/ui";
import {KeyRound, LogOut, Settings as SettingsIcon, ShieldCheck, User as UserIcon} from "lucide-react";

import type {Me} from "@/lib/auth";
import {useLogout} from "@/lib/auth";
import {canSeeAdmin} from "@/lib/nav";
import {hasPermission} from "@/lib/permissions";

export function initials(name: string) {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]!.toUpperCase())
    .join("");
}

/** The signed-in user's menu items — shared by the top-bar avatar and the sidebar identity switcher. */
export function UserMenuItems({ me }: { me: Me }) {
  const logout = useLogout();
  return (
    <>
      <DropdownMenuLabel>
        <div className="flex flex-col">
          <span>{me.display_name}</span>
          <span className="font-mono text-xs font-normal text-ink-3">
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
      {canSeeAdmin(me) && (
        <DropdownMenuItem asChild>
          <Link to="/admin">
            <ShieldCheck />
            Admin
          </Link>
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator />
      <DropdownMenuItem onSelect={() => logout.mutate()} className="text-danger focus:text-danger">
        <LogOut />
        Sign out
      </DropdownMenuItem>
    </>
  );
}

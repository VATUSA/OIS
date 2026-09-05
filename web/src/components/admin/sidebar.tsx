import {Link} from "@tanstack/react-router";
import {KeyRound, LayoutDashboard, type LucideIcon, MessageSquare, ScrollText, ShieldCheck,} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";

type Item = {
  label: string;
  to: string;
  icon: LucideIcon;
  permission?: string;
  exact?: boolean;
};

const NAV: Item[] = [
  { label: "Overview", to: "/admin", icon: LayoutDashboard, exact: true },
  {
    label: "Access Control",
    to: "/admin/access",
    icon: ShieldCheck,
    permission: "access.users.read",
  },
  {
    label: "Audit Log",
    to: "/admin/audit",
    icon: ScrollText,
    permission: "audit.logs.read",
  },
  {
    label: "API Keys",
    to: "/admin/api-keys",
    icon: KeyRound,
    permission: "api_keys.key.read",
  },
  {
    label: "Discord",
    to: "/admin/discord",
    icon: MessageSquare,
    permission: "discord.config.read",
  },
];

export function AdminSidebar() {
  const { data: me } = useMe();
  const items = NAV.filter(
    (item) => !item.permission || hasPermission(me, item.permission),
  );

  return (
    <aside className="h-fit rounded-xl border bg-card p-3">
      <div className="px-2 pb-3 pt-1">
        <p className="font-semibold">Server Admin</p>
        <p className="text-xs text-muted-foreground">
          Signed in as {me?.display_name ?? "…"}
        </p>
      </div>
      <nav className="flex flex-col gap-0.5">
        {items.map((item) => (
          <Link
            key={item.to}
            to={item.to}
            activeOptions={{ exact: item.exact }}
            className="flex items-center gap-2 rounded-md px-2 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground [&.active]:bg-primary [&.active]:text-primary-foreground"
          >
            <item.icon className="size-4" />
            {item.label}
          </Link>
        ))}
      </nav>
    </aside>
  );
}

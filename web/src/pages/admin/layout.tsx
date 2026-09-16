import {Outlet} from "@tanstack/react-router";
import {EmptyState, QueryState} from "@ois/ui";
import {ShieldOff} from "lucide-react";

import {useMe} from "@/lib/auth";
import {canSeeAdmin} from "@/lib/nav";

/**
 * The Admin page (Planning · Historical · Admin). Open to anyone who can use at least one of its
 * links; each link and page stays gated on its own permission. The shell renders the sidebar.
 */
export function AdminLayout() {
  const { data: me, isLoading } = useMe();
  return (
    <QueryState isLoading={isLoading}>
      {canSeeAdmin(me) ? (
        <Outlet />
      ) : (
        <EmptyState icon={ShieldOff} title="No access">
          You don&apos;t have access to any admin pages.
        </EmptyState>
      )}
    </QueryState>
  );
}

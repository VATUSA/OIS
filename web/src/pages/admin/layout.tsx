import {Outlet, useRouterState} from "@tanstack/react-router";
import {EmptyState, QueryState} from "@ois/ui";
import {ShieldOff} from "lucide-react";

import {useMe} from "@/lib/auth";
import {canOpenPath, canSeeAdmin} from "@/lib/nav";

/**
 * The Admin page (Planning · Historical · Admin). Open to anyone who can use at least one of its
 * links; each page is guarded here by its nav item's permission, so a typed URL shows "No access"
 * rather than a page whose every request is refused. The shell renders the sidebar.
 */
export function AdminLayout() {
  const { data: me, isLoading } = useMe();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  return (
    <QueryState isLoading={isLoading}>
      {canOpenPath(me, pathname) ? (
        <Outlet />
      ) : (
        <EmptyState icon={ShieldOff} title="No access">
          {canSeeAdmin(me)
            ? "You don't have access to this page."
            : "You don't have access to any admin pages."}
        </EmptyState>
      )}
    </QueryState>
  );
}

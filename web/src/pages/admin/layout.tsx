import {Outlet} from "@tanstack/react-router";
import {Card, CardContent} from "@ois/ui";

import {AdminSidebar} from "@/components/admin/sidebar";
import {useMe} from "@/lib/auth";
import {isAdmin} from "@/lib/permissions";

export function AdminLayout() {
  const { data: me, isLoading } = useMe();

  if (isLoading) {
    return (
      <div className="py-24 text-center text-muted-foreground">Loading…</div>
    );
  }
  if (!isAdmin(me)) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-muted-foreground">
          You don&apos;t have access to the admin area.
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="grid gap-6 lg:grid-cols-[240px_1fr]">
      <AdminSidebar />
      <div className="min-w-0">
        <Outlet />
      </div>
    </div>
  );
}

import type {Me} from "./auth";

type PermNode = { [key: string]: PermNode | string[] };

/** Whether `me` holds the `segments.action` permission (server admins hold all). */
export function hasPermission(
  me: Me | null | undefined,
  permission: string,
): boolean {
  if (!me) return false;
  if (me.server_admin) return true;

  const parts = permission.split(".");
  const action = parts.pop()!;
  let node: unknown = me.permissions;
  for (const segment of parts) {
    if (!node || typeof node !== "object") return false;
    node = (node as PermNode)[segment];
  }
  return Array.isArray(node) && node.includes(action);
}

/** Permissions that unlock some part of the admin portal. */
export const ADMIN_PERMISSIONS = [
  "access.users.read",
  "audit.logs.read",
  "service_accounts.read",
  "api_keys.key.read",
  "discord.config.read",
  "ace.team.read",
];

/** Whether the user should see the admin portal at all. */
export function isAdmin(me: Me | null | undefined): boolean {
  return (
    !!me && (me.server_admin || ADMIN_PERMISSIONS.some((p) => hasPermission(me, p)))
  );
}

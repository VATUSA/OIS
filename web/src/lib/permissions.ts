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

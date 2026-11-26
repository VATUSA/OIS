import type {QueryClient} from "@tanstack/react-query";
import {redirect} from "@tanstack/react-router";

import {meQueryOptions} from "./auth";
import {canSeeItem, itemForPath} from "./nav";
import {queryClient as appQueryClient} from "./query-client";

/**
 * A route `beforeLoad` that refuses a member who could not see the page's nav link, sending them to
 * the landing page (#1121). The permission is the nav item's own, so the sidebar and the route cannot
 * disagree: a page hidden from the nav cannot be opened by typing its URL either.
 *
 * Navigation only. The backend refuses every request the page would make regardless
 * (`secure-coding.md`). When `/me` cannot be fetched at all (backend unreachable) the page loads and
 * shows the app's own unreachable state.
 *
 * Throws at route-tree build time if `path` has no nav item, so a guard cannot silently guard nothing.
 */
export function navGuard(path: string, client: QueryClient = appQueryClient): () => Promise<void> {
  const hit = itemForPath(path);
  if (!hit || hit.item.to !== path) throw new Error(`navGuard: no nav item for ${path}`);
  const item = hit.item;
  return async () => {
    let me;
    try {
      me = await client.ensureQueryData(meQueryOptions());
    } catch {
      return;
    }
    if (!canSeeItem(me, item)) throw redirect({to: "/"});
  };
}

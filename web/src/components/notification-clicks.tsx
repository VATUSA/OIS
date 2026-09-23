import * as React from "react";
import {useRouter} from "@tanstack/react-router";

import {listenForNotificationClicks} from "@/lib/desktop-notify";

/**
 * Makes a native notification click raise the app and open the page it refers to (#348).
 *
 * Headless, mounted once in the root layout. Nothing happens on the web build — there are no native
 * notifications there to click.
 */
export function NotificationClicks() {
  const router = useRouter();

  React.useEffect(() => {
    let dispose: (() => void) | undefined;
    let cancelled = false;

    void listenForNotificationClicks((route) => {
      // TanStack treats `to` as a pathname and matches it literally against the route tree, so a
      // route carrying its query string ("/ops/tmu?tab=ground-stops") matches nothing and lands the
      // user on not-found. Split it and hand the search over separately, as every other call site
      // in the app does.
      const [to, query] = route.split("?");
      const search = query ? Object.fromEntries(new URLSearchParams(query)) : undefined;
      void router.navigate(
        (search ? { to, search } : { to }) as Parameters<typeof router.navigate>[0],
      );
    }).then((d) => {
      // Unmounted before the listener finished registering — tear it straight back down.
      if (cancelled) d?.();
      else dispose = d;
    });

    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [router]);

  return null;
}

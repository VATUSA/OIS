import * as React from "react";
import {useRouter} from "@tanstack/react-router";

import {safeUnlisten} from "@/lib/desktop-events";
import {listenForNotificationClicks} from "@/lib/desktop-notify";

/**
 * Splits a notification's route into what `router.navigate` takes.
 *
 * TanStack treats `to` as a pathname and matches it literally against the route tree, so a route
 * carrying its query string ("/ops/tmu?tab=ground-stops") matches nothing and lands the user on
 * not-found. The search goes over separately, as every other call site in the app passes it.
 */
export function toNavigation(route: string): {to: string; search?: Record<string, string>} {
  const [to = route, query] = route.split("?");
  return query ? {to, search: Object.fromEntries(new URLSearchParams(query))} : {to};
}

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
      void router.navigate(toNavigation(route) as Parameters<typeof router.navigate>[0]);
    }).then((d) => {
      // Unmounted before the listener finished registering — tear it straight back down.
      if (cancelled) safeUnlisten(d);
      else dispose = d;
    });

    return () => {
      cancelled = true;
      safeUnlisten(dispose);
    };
  }, [router]);

  return null;
}

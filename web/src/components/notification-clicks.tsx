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
      void router.navigate({ to: route });
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

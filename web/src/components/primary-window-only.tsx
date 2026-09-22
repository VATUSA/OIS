import * as React from "react";

import {isMainWindow, isTauri} from "@/lib/platform";

/**
 * Renders its children only in the app's primary window (#350).
 *
 * Route windows run the full shell, so without this every open window mounts its own copy of the
 * notification stack and the realtime socket: one ground stop fires a native notification per
 * window, clicking one makes every window raise and navigate itself, and each holds its own
 * websocket. All of that should happen once, in the window that owns it.
 *
 * On the web build there is only ever one window, so children render as they always did.
 */
export function PrimaryWindowOnly({children}: {children: React.ReactNode}) {
  // Start `true` off-desktop so the web build never blinks its shell out while a check resolves.
  const [primary, setPrimary] = React.useState(!isTauri());

  React.useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    void isMainWindow().then((main) => {
      if (!cancelled) setPrimary(main);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return primary ? <>{children}</> : null;
}

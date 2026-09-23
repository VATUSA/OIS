import * as React from "react";

import {isMainWindow, isTauri} from "@/lib/platform";

/**
 * Renders its children only in the app's primary window (#350).
 *
 * Route windows run the full shell, so without this every open window mounts its own copy of the
 * native-notification stack: one ground stop fires a notification per window, and clicking one
 * makes every window raise and navigate itself. That should happen once, in the main window.
 *
 * Only OS-level side effects belong in here. The realtime socket deliberately stays per window —
 * each webview has its own query cache, so one socket in the main window could not keep the
 * others live — and so do in-app toasts and alerts, which are only seen in the window they fire in.
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

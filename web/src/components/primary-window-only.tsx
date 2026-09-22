import * as React from "react";

import {isMainWindow, isTauri} from "@/lib/platform";

/**
 * Renders its children only in the app's primary window.
 *
 * Route windows (#350) run the full shell, so anything OS-global mounted in `RootLayout` otherwise
 * runs once per open window: a native notification per window (#348), and a tray that every window
 * races to rebuild (#351). In-app UI — toasts, alerts, modals — deliberately does *not* belong in
 * here, because those are only visible in the window they fire in.
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

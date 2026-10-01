import {useRouterState} from "@tanstack/react-router";
import {Button} from "@ois/ui";
import {AppWindow} from "lucide-react";

import {can} from "@/lib/platform";
import {openRouteWindow} from "@/lib/popout";

/**
 * Opens the page you're on in its own native window, so the app can be spread across monitors
 * (#350) — the facility map on one screen, IDST on another.
 *
 * Desktop only; there is nothing to open on the web. The window id is the route itself, so asking
 * twice raises the window that already exists rather than stacking duplicates.
 */
export function OpenInWindowButton() {
  const location = useRouterState({select: (s) => s.location});
  const title = useRouterState({
    select: (s) => s.matches.at(-1)?.staticData?.title,
  });

  // The window the app already lives in is not worth "opening" again.
  if (location.pathname === "/") return null;
  // Gate on the ability, not just the platform — platform.ts's own rule.
  if (!can("multiWindow")) return null;

  return (
    <Button
      size="icon"
      variant="ghost"
      className="ml-auto size-7 text-ink-3 hover:text-ink"
      title="Open this page in its own window"
      onClick={() =>
        void openRouteWindow({
          id: location.pathname,
          title: title ? `OIS · ${title}` : "OIS",
          route: `${location.pathname}${location.searchStr ?? ""}`,
        })
      }
    >
      <AppWindow className="size-4" />
    </Button>
  );
}

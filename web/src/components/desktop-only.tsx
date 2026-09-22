import * as React from "react";

import {isTauri} from "@/lib/platform";

/**
 * Renders its children only in the Tauri desktop shell; on the web build it renders `fallback`,
 * or nothing.
 *
 * This is the UI half of the platform seam (`@/lib/platform`) — use it for chrome that only makes
 * sense natively (tray controls, window management, a pop-out button). When the gate is about a
 * specific ability rather than the platform itself, prefer `can("tray")` so the UI appears only
 * once that feature actually exists.
 */
export function DesktopOnly({
  children,
  fallback = null,
}: {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}) {
  return <>{isTauri() ? children : fallback}</>;
}

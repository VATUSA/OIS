import * as React from "react";
import {Button} from "@ois/ui";
import {Minus, Square, Copy, X} from "lucide-react";

import {can} from "@/lib/platform";

/**
 * Minimize / maximize / close for the frameless main window (#402).
 *
 * The native title bar is off (`decorations: false`), so these are the only way to manage the window
 * short of OS shortcuts — which is why they sit in the breadcrumb row rather than the sidebar's
 * chrome row: that row collapses to a 60px rail, and controls that move (or vanish) when you collapse
 * a sidebar are worse than controls in a slightly different place from the back/forward pair.
 *
 * Desktop only, and gated on the ability rather than the platform, per `platform.ts`'s own rule.
 * `@tauri-apps/*` is reached through a dynamic `import()` — a static one is banned repo-wide by
 * `eslint.config.mjs` because it would pull Tauri into the bundle every browser downloads.
 */
async function mainWindow() {
  const {getCurrentWindow} = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

function ControlButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Button
      size="icon"
      variant="ghost"
      aria-label={label}
      title={label}
      className="size-7 text-ink-3 hover:text-ink"
      // The row around these is the window's drag region; without this a click on a control would
      // start dragging the window instead of pressing the button.
      data-tauri-drag-region="false"
      onClick={onClick}
    >
      {children}
    </Button>
  );
}

export function WindowControls() {
  const [maximized, setMaximized] = React.useState(false);

  // Gate before any effect so the web build neither renders nor subscribes.
  const enabled = can("windowControls");

  React.useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const sync = async () => {
      try {
        const win = await mainWindow();
        const now = await win.isMaximized();
        if (alive) setMaximized(now);
      } catch {
        // Can't tell; the icon is cosmetic and the toggle still works.
      }
    };
    void sync();
    return () => {
      alive = false;
    };
  }, [enabled]);

  if (!enabled) return null;

  const act = (run: (win: Awaited<ReturnType<typeof mainWindow>>) => Promise<unknown>) => () => {
    void (async () => {
      try {
        const win = await mainWindow();
        await run(win);
        // Re-read rather than assume: a toggle the OS refused would otherwise flip the icon anyway.
        setMaximized(await win.isMaximized());
      } catch {
        // Failing to manage the window must not break the page it is drawn on.
      }
    })();
  };

  return (
    <div className="flex items-center gap-0.5">
      <ControlButton label="Minimize" onClick={act((win) => win.minimize())}>
        <Minus className="size-4" />
      </ControlButton>
      <ControlButton
        label={maximized ? "Restore" : "Maximize"}
        onClick={act((win) => win.toggleMaximize())}
      >
        {maximized ? <Copy className="size-3.5" /> : <Square className="size-3.5" />}
      </ControlButton>
      <ControlButton label="Close" onClick={act((win) => win.close())}>
        <X className="size-4" />
      </ControlButton>
    </div>
  );
}

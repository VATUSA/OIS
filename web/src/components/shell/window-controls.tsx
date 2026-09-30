import * as React from "react";
import {Button} from "@ois/ui";
import {Minus, Square, Copy, X} from "lucide-react";

import {safeUnlisten} from "@/lib/desktop-events";
import {can, MAIN_WINDOW_LABEL} from "@/lib/platform";

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

/**
 * Whether *this* webview is the frameless window — which is the main window and only the main window.
 *
 * `can("windowControls")` is true in every Tauri webview, but `decorations: false` is set on `main`
 * alone. Route windows (#350) render this same shell inside a window that still has its native title
 * bar, so a capability-only gate drew a second set of controls on top of the OS's and turned that
 * row into a drag region the window did not need (#402 review). `platform.ts` warns about exactly
 * this: every window loads the same bundle.
 *
 * Starts `false` and resolves, so the controls appear a tick late rather than appearing in a window
 * that should not have them.
 */
export function useFramelessWindow(): boolean {
  const enabled = can("windowControls");
  const [frameless, setFrameless] = React.useState(false);

  React.useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void (async () => {
      try {
        const win = await mainWindow();
        if (alive) setFrameless(win.label === MAIN_WINDOW_LABEL);
      } catch {
        // Can't tell which window this is — draw nothing rather than risk duplicating the OS's.
      }
    })();
    return () => {
      alive = false;
    };
  }, [enabled]);

  return enabled && frameless;
}

/**
 * What makes the app's top bar move the frameless window, or `undefined` everywhere else.
 *
 * Only the drag region: Tauri's own `drag.js` is injected into every webview and already maximizes
 * on a double-click of a drag region, handling the macOS/Windows difference itself. An extra
 * `onDoubleClick` alongside it toggled maximize *twice* — dead on Windows, unrestorable on macOS
 * (#402 review) — so the built-in is the only double-click here.
 */
export function useDragRegionProps(): {readonly "data-tauri-drag-region": true} | undefined {
  return useFramelessWindow() ? ({"data-tauri-drag-region": true} as const) : undefined;
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
  const frameless = useFramelessWindow();
  const [maximized, setMaximized] = React.useState(false);

  React.useEffect(() => {
    if (!frameless) return;
    let alive = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        const win = await mainWindow();
        const read = async () => {
          const now = await win.isMaximized();
          if (alive) setMaximized(now);
        };
        await read();
        // The window is maximized and restored without us too — Snap, Win+Up, dragging to the top
        // edge, double-clicking the bar — and the toggle's label is its accessible name, so a
        // read-once state tells a screen reader the wrong action (#402 review).
        const stop = await win.onResized(() => void read());
        if (alive) unlisten = stop;
        else safeUnlisten(stop);
      } catch {
        // Can't tell; the icon is cosmetic and the toggle still works.
      }
    })();

    return () => {
      alive = false;
      safeUnlisten(unlisten);
    };
  }, [frameless]);

  if (!frameless) return null;

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

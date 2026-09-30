import * as React from "react";
import {Minus, Plus, X} from "lucide-react";

import {can, isMacOS, isMainWindow} from "@/lib/platform";

/**
 * The main window's own minimize / zoom / close, and the drag region that moves it (#402, #419).
 *
 * Which of those the app draws depends on the host OS, because the window is shaped differently on
 * each: macOS keeps its decorations (`tauri.macos.conf.json` — an `Overlay` title bar with a
 * `trafficLightPosition` that insets the real buttons into the app's own chrome row), so the OS draws
 * the traffic lights and the app must not. Windows and Linux run `decorations: false`, so the app
 * draws a replica of those buttons in the same place.
 *
 * The drag region is needed on **every** platform: an `Overlay` title bar is transparent and sits over
 * the content, so without it the top of a macOS window would not move the window either.
 *
 * `@tauri-apps/*` is reached through a dynamic `import()` — a static one is banned repo-wide by
 * `eslint.config.mjs` because it would pull Tauri into the bundle every browser downloads.
 */
async function currentWindow() {
  const {getCurrentWindow} = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

/**
 * Whether this window's top-left carries window buttons — the OS's on macOS, ours everywhere else.
 *
 * The sidebar asks this to reserve the space they sit in, which has to happen on *both* platforms:
 * the native macOS lights are drawn by the OS over whatever the page put there, so the room must be
 * left for them exactly as it is for the replica.
 *
 * It is the same question as "is this the app's main window", because that is the only window the app
 * shapes.
 *
 * `can("windowControls")` is true in every Tauri webview, but the shaping in `tauri.conf.json` applies
 * to `main` alone. Route windows (#350) render this same shell inside a window that still has its own
 * native title bar, so a capability-only gate drew a second set of controls on top of the OS's and
 * turned that row into a drag region the window did not need (#402 review). `platform.ts` warns about
 * exactly this: every window loads the same bundle.
 *
 * Starts `false` and resolves, so chrome appears a tick late rather than appearing in a window that
 * should not have it. `isMainWindow()` answers `false` when the window cannot be read, which is the
 * answer this gate wants anyway.
 */
export function useWindowChrome(): boolean {
  const enabled = can("windowControls");
  const [main, setMain] = React.useState(false);

  React.useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void (async () => {
      const isMain = await isMainWindow();
      if (alive) setMain(isMain);
    })();
    return () => {
      alive = false;
    };
  }, [enabled]);

  return enabled && main;
}

/**
 * What makes the app's chrome row move the window, or `undefined` in a browser and in a route window.
 *
 * Only the drag region: Tauri's own `drag.js` is injected into every webview and already maximizes
 * on a double-click of a drag region, handling the macOS/Windows difference itself. An extra
 * `onDoubleClick` alongside it toggled maximize *twice* — dead on Windows, unrestorable on macOS
 * (#402 review) — so the built-in is the only double-click here.
 */
export function useDragRegionProps(): {readonly "data-tauri-drag-region": true} | undefined {
  return useWindowChrome() ? ({"data-tauri-drag-region": true} as const) : undefined;
}

/**
 * Whether the window has focus, so the replica greys out the way the real buttons do.
 *
 * Starts focused: a window drawing its own chrome is almost always the one being looked at, and
 * guessing "focused" wrong for a tick is less visible than starting grey and lighting up.
 */
function useWindowFocus(enabled: boolean): boolean {
  const [focused, setFocused] = React.useState(true);

  React.useEffect(() => {
    if (!enabled) return;
    let alive = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        const win = await currentWindow();
        const now = await win.isFocused();
        if (alive) setFocused(now);
        const stop = await win.onFocusChanged(({payload}) => {
          if (alive) setFocused(payload);
        });
        if (alive) unlisten = stop;
        else stop();
      } catch {
        // Can't tell; the buttons stay in their focused colours and still work.
      }
    })();

    return () => {
      alive = false;
      unlisten?.();
    };
  }, [enabled]);

  return focused;
}

/**
 * One traffic light: a 12px dot that reveals its glyph when the group is hovered, as macOS's own
 * buttons do — the glyphs appear on all three at once, not only the one under the cursor.
 *
 * Colours come from tokens (`--traffic-*` in `globals.css`), not inline hex, per DESIGN.md. They are
 * deliberately *not* the semantic status colours: these are Apple's three window buttons, and reusing
 * `--danger` for the close dot would make a status token mean "chrome".
 */
function TrafficLight({
  label,
  tone,
  onClick,
  children,
}: {
  label: string;
  tone: "close" | "minimize" | "zoom";
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-tone={tone}
      // The row around these is the window's drag region; without this a click on a control would
      // start dragging the window instead of pressing the button.
      data-tauri-drag-region="false"
      onClick={onClick}
      className="traffic-light"
    >
      <span aria-hidden="true">{children}</span>
    </button>
  );
}

/**
 * The traffic-light replica, for the windows whose OS does not draw its own.
 *
 * Renders nothing on macOS — the real buttons are already there, in this same spot, put there by
 * `trafficLightPosition` — and nothing in a browser or in a route window.
 */
export function WindowControls() {
  const main = useWindowChrome();
  const drawn = main && !isMacOS();
  const focused = useWindowFocus(drawn);

  const act = (run: (win: Awaited<ReturnType<typeof currentWindow>>) => Promise<unknown>) => () => {
    void (async () => {
      try {
        await run(await currentWindow());
      } catch {
        // Failing to manage the window must not break the page it is drawn on.
      }
    })();
  };

  if (!drawn) return null;

  return (
    <div className="traffic-lights" data-blurred={focused ? undefined : true}>
      <TrafficLight label="Close" tone="close" onClick={act((win) => win.close())}>
        <X strokeWidth={4} />
      </TrafficLight>
      <TrafficLight label="Minimize" tone="minimize" onClick={act((win) => win.minimize())}>
        <Minus strokeWidth={4} />
      </TrafficLight>
      {/* macOS calls this zoom, not maximize, and its glyph doesn't change with the window's state —
          which is also why nothing here reads `isMaximized()`: there is no label to keep truthful. */}
      <TrafficLight label="Zoom" tone="zoom" onClick={act((win) => win.toggleMaximize())}>
        <Plus strokeWidth={4} />
      </TrafficLight>
    </div>
  );
}

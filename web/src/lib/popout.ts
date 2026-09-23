import {can} from "@/lib/platform";
import {clampToMonitors, toLogical, type Monitor, type Rect} from "@/lib/popout-geometry";
import {forgetWindow, rememberWindow, rememberedWindows} from "@/lib/window-registry";

/**
 * Pop-out mini-windows (#349) — small always-on-top native windows holding one panel, so a
 * controller can keep a metering ladder or a widget over CRC, vATIS or charts.
 *
 * A pop-out is just the app at a bare route: `?embed=1` (`web/src/router.tsx`) already strips the
 * shell, and a *relative* URL resolves against the dev server in development and the asset protocol
 * in production, so there is no environment branching here.
 *
 * It does not share the main window's react-query cache — each Tauri window is its own webview, so
 * it can't. It doesn't need to: the pop-out mounts the same providers and the same realtime socket,
 * and the server is the source of truth, so both windows move in step on the same nudges.
 *
 * This is also the window plumbing multi-window (#350) builds on, which is why opening is keyed on
 * a caller-supplied id rather than hardcoded to panels.
 */

export type PopoutSpec = {
  /** Stable per-panel id. Becomes the window label, so reopening the same panel raises it. */
  id: string;
  title: string;
  /** App-relative route, e.g. `/popout/fca/abc`. `?embed=1` is added here. */
  route: string;
  width?: number;
  height?: number;
};

/** What differs between a floating panel and a full route window; everything else is shared. */
type WindowKind = {
  labelPrefix: string;
  /** Strip the shell (`?embed=1`)? A panel wants just itself; a route window wants the whole app. */
  embed: boolean;
  alwaysOnTop: boolean;
  defaultSize: {width: number; height: number};
  minSize: {width: number; height: number};
};

/** A detached panel: small, bare, and pinned above other applications (#349). */
const PANEL: WindowKind = {
  labelPrefix: "popout-",
  embed: true,
  alwaysOnTop: true,
  defaultSize: {width: 380, height: 520},
  // The panels we allow out declare minimums around 200-350px.
  minSize: {width: 260, height: 200},
};

/** A whole route on another monitor: the full app, behaving like any other window (#350). */
const ROUTE: WindowKind = {
  labelPrefix: "window-",
  embed: false,
  alwaysOnTop: false,
  defaultSize: {width: 1200, height: 800},
  // The shell itself needs room; below this the sidebar and content stop making sense.
  minSize: {width: 720, height: 480},
};

/** How long a drag or resize must be still before the new geometry is written. */
const GEOMETRY_SETTLE_MS = 300;

/**
 * Window labels must be simple; ids can be UUIDs or route paths, so normalise.
 *
 * A plain substitution is not enough on its own: `/ops/idst` and `/ops-idst` both flatten to
 * `-ops-idst`, and since the label is what `getByLabel` raises, one route would surface the other's
 * window. Anything that had to be rewritten therefore carries a short hash of the original id, so
 * distinct ids stay distinct while the label stays readable in the common case.
 */
function labelFor(kind: WindowKind, id: string): string {
  const flattened = id.replace(/[^a-zA-Z0-9-]/g, "-");
  if (flattened === id) return `${kind.labelPrefix}${id}`;

  // djb2 — a label disambiguator, not a security boundary.
  let hash = 5381;
  for (let i = 0; i < id.length; i += 1) hash = ((hash << 5) + hash + id.charCodeAt(i)) >>> 0;
  return `${kind.labelPrefix}${flattened}-${hash.toString(36)}`;
}

/** The window label a detached panel uses. */
export function popoutLabel(id: string): string {
  return labelFor(PANEL, id);
}

/** The window label a route window uses. */
export function routeWindowLabel(id: string): string {
  return labelFor(ROUTE, id);
}

/**
 * Keyed by window *label*, not by id: a panel and a route window may legitimately share an id
 * (the same FCA, popped out and opened as a route), and they are different windows with different
 * sizes. Keying on the label keeps their geometry apart.
 */
function storageKey(label: string): string {
  return `ois.window.${label}`;
}

/** Geometry is per machine, not per user: a position that suits a three-monitor desk is wrong on a laptop. */
function readGeometry(label: string): Rect | undefined {
  try {
    const raw = localStorage.getItem(storageKey(label));
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as Partial<Rect>;
    const {x, y, width, height} = parsed;
    if ([x, y, width, height].some((n) => typeof n !== "number" || !Number.isFinite(n))) {
      return undefined;
    }
    return {x: x!, y: y!, width: width!, height: height!};
  } catch {
    // Private mode, cleared site data, or something else wrote nonsense here. Let the OS place it.
    return undefined;
  }
}

function writeGeometry(label: string, rect: Rect) {
  try {
    localStorage.setItem(storageKey(label), JSON.stringify(rect));
  } catch {
    // Not being able to remember where a window was is not worth surfacing.
  }
}

/**
 * Opens a panel in its own always-on-top window, or raises it if it is already open.
 *
 * Raising rather than duplicating matters: clicking "pop out" twice should not leave two identical
 * windows fighting for the same screen space.
 *
 * A no-op on the web build, where `can("miniWindows")` is false.
 */
export async function openPopout(spec: PopoutSpec): Promise<boolean> {
  if (!can("miniWindows")) return false;
  return openWindow(PANEL, spec);
}

/**
 * Opens a whole route in its own window, so the app can be spread across monitors (#350).
 *
 * Unlike a pop-out this is an ordinary window — full shell, not pinned on top — because it is the
 * app, not a floating readout. It authenticates without a second sign-in: the token lives in the OS
 * keychain (#346), not a per-window cookie.
 *
 * A no-op on the web build, where `can("multiWindow")` is false.
 */
export async function openRouteWindow(spec: PopoutSpec): Promise<boolean> {
  if (!can("multiWindow")) return false;
  return openWindow(ROUTE, spec);
}

async function openWindow(kind: WindowKind, spec: PopoutSpec): Promise<boolean> {
  const label = labelFor(kind, spec.id);
  const remembered = kind === ROUTE;

  try {
    const {WebviewWindow} = await import("@tauri-apps/api/webviewWindow");
    const {availableMonitors} = await import("@tauri-apps/api/window");

    const existing = await WebviewWindow.getByLabel(label);
    if (existing) {
      await existing.unminimize().catch(() => undefined);
      await existing.show();
      await existing.setFocus();
      return true;
    }

    const monitors = (await availableMonitors()) as unknown as Monitor[];
    const restored = clampToMonitors(readGeometry(label), monitors);
    const size = {
      width: restored?.width ?? spec.width ?? kind.defaultSize.width,
      height: restored?.height ?? spec.height ?? kind.defaultSize.height,
    };

    const separator = spec.route.includes("?") ? "&" : "?";
    const win = new WebviewWindow(label, {
      url: kind.embed ? `${spec.route}${separator}embed=1` : spec.route,
      title: spec.title,
      width: size.width,
      height: size.height,
      ...(restored ? {x: restored.x, y: restored.y} : {}),
      alwaysOnTop: kind.alwaysOnTop,
      resizable: true,
      minWidth: kind.minSize.width,
      minHeight: kind.minSize.height,
    });

    // Remember where the user put it. Both events are needed — a move and a resize are separate —
    // and both fire continuously while dragging, so the write is debounced: otherwise a single
    // drag across the screen would mean hundreds of synchronous localStorage writes.
    let settle: number | undefined;
    const remember = () => {
      window.clearTimeout(settle);
      settle = window.setTimeout(async () => {
        try {
          // Tauri reports these in physical pixels; stored geometry is logical (see `Rect`).
          const [position, outer, factor] = await Promise.all([
            win.outerPosition(),
            win.outerSize(),
            win.scaleFactor(),
          ]);
          writeGeometry(
            label,
            toLogical({x: position.x, y: position.y, width: outer.width, height: outer.height}, factor),
          );
        } catch {
          // The window is probably closing; nothing to remember.
        }
      }, GEOMETRY_SETTLE_MS);
    };

    await win.onMoved(remember);
    await win.onResized(remember);

    if (remembered) {
      rememberWindow({id: spec.id, route: spec.route, title: spec.title});
      // Closing a window is how the user says "not next time", so that has to stick.
      await win.onCloseRequested(() => forgetWindow(spec.id));
    }

    return true;
  } catch {
    return false;
  }
}

/** Closes a pop-out if it's open. Closing one that isn't is not an error. */
export async function closePopout(id: string): Promise<void> {
  if (!can("miniWindows")) return;

  try {
    const {WebviewWindow} = await import("@tauri-apps/api/webviewWindow");
    const win = await WebviewWindow.getByLabel(popoutLabel(id));
    await win?.close();
  } catch {
    // Already gone.
  }
}

/**
 * Reopens the windows that were open when the app last closed (#350).
 *
 * **Only the main window does this.** Without that guard every restored window would restore the
 * whole set again as it booted, and one relaunch would spawn windows without end.
 *
 * Not awaited by the caller: a window failing to reopen must not hold up first paint.
 */
export async function restoreWindows(): Promise<number> {
  if (!can("multiWindow")) return 0;

  try {
    const {getCurrentWindow} = await import("@tauri-apps/api/window");
    if (getCurrentWindow().label !== "main") return 0;
  } catch {
    return 0;
  }

  // One at a time, not in parallel: window placement is deterministic this way, and opening a
  // handful of native windows simultaneously is not something to ask a window manager to do at
  // launch. There are only ever a few.
  const windows = rememberedWindows();
  for (const win of windows) await openRouteWindow(win);
  return windows.length;
}

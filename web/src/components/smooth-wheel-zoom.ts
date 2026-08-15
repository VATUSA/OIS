import L from "leaflet";

/**
 * Continuous, eased wheel/trackpad zoom — replaces Leaflet's default scroll-wheel handler.
 *
 * The default handler fires a fresh ~250ms zoom *animation* for each debounced burst of
 * wheel events; on a trackpad the bursts overlap and keep interrupting each other, which is
 * the stuttery, jarring feel. Here a wheel event only nudges a *target* zoom, and a single
 * requestAnimationFrame loop eases the live zoom toward it via `map._move` (no per-step
 * animation), keeping the point under the cursor fixed — so a trackpad glides the way
 * VATSIM Radar / Google Maps do. Mirrors the well-known Leaflet.SmoothWheelZoom approach.
 */
type SmoothOpts = {
  /** Zoom levels per pixel of scroll delta. Higher = more sensitive. */
  sensitivity?: number;
  /** Fraction of the remaining gap closed each frame (0–1); lower = smoother/slower. */
  ease?: number;
};

export function enableSmoothWheelZoom(map: L.Map, opts: SmoothOpts = {}) {
  const sensitivity = opts.sensitivity ?? 0.0022;
  const ease = opts.ease ?? 0.2;
  const anyMap = map as any;
  const container = map.getContainer();

  // Take over from the built-in handler.
  map.scrollWheelZoom.disable();

  let running = false;
  let goalZoom = map.getZoom();
  let liveZoom = map.getZoom();
  let anchorPoint = map.getSize().divideBy(2);
  let anchorLatLng = map.getCenter();
  let frame = 0;
  let endTimer: ReturnType<typeof setTimeout> | undefined;

  function step() {
    liveZoom += (goalZoom - liveZoom) * ease;
    if (Math.abs(goalZoom - liveZoom) < 0.002) liveZoom = goalZoom;

    // Keep the latLng under the cursor pinned to the same pixel at the new zoom.
    const offset = anchorPoint.subtract(map.getSize().divideBy(2));
    const center = map.unproject(
      map.project(anchorLatLng, liveZoom).subtract(offset),
      liveZoom,
    );
    anyMap._move(center, liveZoom, { pinch: true });

    if (liveZoom !== goalZoom) {
      frame = requestAnimationFrame(step);
    } else {
      running = false;
    }
  }

  function end() {
    cancelAnimationFrame(frame);
    if (running || liveZoom !== goalZoom) {
      liveZoom = goalZoom;
      step();
    }
    running = false;
    anyMap._moveEnd(true);
  }

  function onWheel(e: WheelEvent) {
    // Normalize away deltaMode (some mice report lines/pages, not pixels).
    let delta = e.deltaY;
    if (e.deltaMode === 1) delta *= 20;
    else if (e.deltaMode === 2) delta *= 60;

    if (!running) {
      goalZoom = map.getZoom();
      liveZoom = map.getZoom();
    }
    goalZoom = anyMap._limitZoom(goalZoom - delta * sensitivity);
    anchorPoint = map.mouseEventToContainerPoint(e);
    anchorLatLng = map.containerPointToLatLng(anchorPoint);

    if (!running) {
      running = true;
      anyMap._stop();
      frame = requestAnimationFrame(step);
    }
    clearTimeout(endTimer);
    endTimer = setTimeout(end, 180);

    e.preventDefault();
    e.stopPropagation();
  }

  // Non-passive so preventDefault stops the page from scrolling.
  container.addEventListener("wheel", onWheel, { passive: false });
  map.on("unload", () => {
    container.removeEventListener("wheel", onWheel);
    clearTimeout(endTimer);
    cancelAnimationFrame(frame);
  });
}

import L from "leaflet";

import {aircraftIconUrl} from "@/lib/aircraft-icons";

/** One aircraft to draw on the canvas layer. */
export type CanvasAircraft = {
  callsign: string;
  lat: number;
  lon: number;
  heading: number;
  actype: string;
  dep: string;
  arr: string;
  alt: number;
  gs: number;
};

type Opts = {
  /** Called when an aircraft is clicked (hit-tested against the canvas). */
  onClick?: (callsign: string) => void;
  /** Called on hover — the aircraft under the cursor, or null when none. */
  onHover?: (ac: CanvasAircraft | null) => void;
};

// Rasterized icon cache, shared across layers/instances. A URL maps to its <img>;
// drawing waits until the image has decoded, then a redraw is requested.
const imgCache = new Map<string, HTMLImageElement>();
function iconImage(url: string, onReady: () => void): HTMLImageElement | null {
  const cached = imgCache.get(url);
  if (cached) return cached.complete && cached.naturalWidth > 0 ? cached : null;
  const img = new Image();
  img.onload = onReady;
  img.src = url;
  imgCache.set(url, img);
  return null;
}

const ICON_SIZE = 22;
const HIT_RADIUS_SQ = 13 * 13;

/**
 * Draws all live aircraft to a single `<canvas>` (one layer, not ~1000 DOM markers) — the
 * OpenLayers/VATSIM-Radar approach done inside Leaflet. The canvas rides Leaflet's pane
 * transform during pan and its zoom-animation transform during zoom (so it stays smooth), and
 * repaints crisply on `moveend`/`zoomend`. Hover/click are hit-tested against the last paint.
 */
const AircraftCanvasLayer = L.Layer.extend({
  initialize(this: any, opts: Opts) {
    this._opts = opts ?? {};
    this._data = [] as CanvasAircraft[];
    this._exclude = new Set<string>();
    this._planeIcons = false;
    this._positions = [] as { ac: CanvasAircraft; x: number; y: number }[];
  },

  /** Replace the aircraft set. `exclude` are callsigns drawn by another layer (matched). */
  setData(
    this: any,
    data: CanvasAircraft[],
    exclude: Set<string>,
    planeIcons: boolean,
  ) {
    this._data = data;
    this._exclude = exclude;
    this._planeIcons = planeIcons;
    if (this._map) this._redraw();
  },

  onAdd(this: any, map: L.Map) {
    this._map = map;
    const canvas = (this._canvas = L.DomUtil.create(
      "canvas",
      "leaflet-layer",
    ) as HTMLCanvasElement);
    canvas.style.pointerEvents = "none"; // the map handles clicks; we hit-test ourselves
    const size = map.getSize();
    canvas.width = size.x;
    canvas.height = size.y;
    const animated = map.options.zoomAnimation && L.Browser.any3d;
    L.DomUtil.addClass(canvas, `leaflet-zoom-${animated ? "animated" : "hide"}`);

    // Own pane, above the vector overlays (FCA lines / routes) but below marker labels.
    if (!map.getPane("aircraftCanvas")) {
      const pane = map.createPane("aircraftCanvas");
      pane.style.zIndex = "450";
    }
    map.getPane("aircraftCanvas")!.appendChild(canvas);

    map.on("moveend zoomend viewreset resize", this._reset, this);
    if (animated) map.on("zoomanim", this._animateZoom, this);
    map.on("click", this._onClick, this);
    map.on("mousemove", this._onMove, this);
    this._reset();
  },

  onRemove(this: any, map: L.Map) {
    L.DomUtil.remove(this._canvas);
    map.off("moveend zoomend viewreset resize", this._reset, this);
    map.off("zoomanim", this._animateZoom, this);
    map.off("click", this._onClick, this);
    map.off("mousemove", this._onMove, this);
  },

  _reset(this: any) {
    const size = this._map.getSize();
    if (this._canvas.width !== size.x) this._canvas.width = size.x;
    if (this._canvas.height !== size.y) this._canvas.height = size.y;
    const topLeft = this._map.containerPointToLayerPoint([0, 0]);
    L.DomUtil.setPosition(this._canvas, topLeft);
    this._redraw();
  },

  // Mirror L.Canvas: scale + shift the already-drawn canvas to track the zoom animation.
  _animateZoom(this: any, e: L.ZoomAnimEvent) {
    const scale = this._map.getZoomScale(e.zoom, this._map.getZoom());
    const offset = (this._map as any)._latLngBoundsToNewLayerBounds(
      this._map.getBounds(),
      e.zoom,
      e.center,
    ).min;
    L.DomUtil.setTransform(this._canvas, offset, scale);
  },

  // World-copy longitude offsets covering the current view (so aircraft repeat as you scroll).
  _offsets(this: any): number[] {
    const b = this._map.getBounds();
    const start = Math.floor((b.getWest() + 180) / 360);
    const end = Math.ceil((b.getEast() - 180) / 360);
    const offs: number[] = [];
    for (let i = start; i <= end; i++) offs.push(i * 360);
    return offs.length ? offs : [0];
  },

  _redraw(this: any) {
    const map = this._map;
    const canvas = this._canvas as HTMLCanvasElement;
    const ctx = canvas.getContext("2d")!;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    const topLeft = map.containerPointToLayerPoint([0, 0]);
    const offsets = this._offsets();
    const positions: { ac: CanvasAircraft; x: number; y: number }[] = [];
    for (const ac of this._data as CanvasAircraft[]) {
      if (this._exclude.has(ac.callsign)) continue;
      for (const off of offsets) {
        const lp = map
          .latLngToLayerPoint([ac.lat, ac.lon + off])
          .subtract(topLeft);
        if (
          lp.x < -ICON_SIZE ||
          lp.x > canvas.width + ICON_SIZE ||
          lp.y < -ICON_SIZE ||
          lp.y > canvas.height + ICON_SIZE
        ) {
          continue; // off-screen on this world copy
        }
        this._drawOne(ctx, lp.x, lp.y, ac);
        positions.push({ ac, x: lp.x, y: lp.y });
      }
    }
    this._positions = positions;
  },

  _drawOne(
    this: any,
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    ac: CanvasAircraft,
  ) {
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate((ac.heading * Math.PI) / 180);
    if (this._planeIcons) {
      const img = iconImage(aircraftIconUrl(ac.actype), () => this._redraw());
      if (img) {
        ctx.drawImage(img, -ICON_SIZE / 2, -ICON_SIZE / 2, ICON_SIZE, ICON_SIZE);
      } else {
        drawTriangle(ctx); // fallback until the sprite decodes
      }
    } else {
      drawTriangle(ctx);
    }
    ctx.restore();
  },

  _hitTest(this: any, cp: L.Point): CanvasAircraft | null {
    let best: CanvasAircraft | null = null;
    let bestDist = HIT_RADIUS_SQ;
    for (const p of this._positions as {
      ac: CanvasAircraft;
      x: number;
      y: number;
    }[]) {
      const dx = p.x - cp.x;
      const dy = p.y - cp.y;
      const d = dx * dx + dy * dy;
      if (d < bestDist) {
        bestDist = d;
        best = p.ac;
      }
    }
    return best;
  },

  _onClick(this: any, e: L.LeafletMouseEvent) {
    const hit = this._hitTest(e.containerPoint);
    if (hit) this._opts.onClick?.(hit.callsign);
  },

  _onMove(this: any, e: L.LeafletMouseEvent) {
    const hit = this._hitTest(e.containerPoint);
    if (hit !== this._hovered) {
      this._hovered = hit;
      this._map.getContainer().style.cursor = hit ? "pointer" : "";
      this._opts.onHover?.(hit);
    }
  },
});

/** A small nose-up arrow (the plain-traffic glyph), drawn at the canvas origin. */
function drawTriangle(ctx: CanvasRenderingContext2D) {
  ctx.beginPath();
  ctx.moveTo(0, -6);
  ctx.lineTo(4.5, 5);
  ctx.lineTo(0, 2.5);
  ctx.lineTo(-4.5, 5);
  ctx.closePath();
  ctx.fillStyle = "rgba(34, 211, 238, 0.85)";
  ctx.fill();
}

export function aircraftCanvasLayer(opts: Opts): L.Layer {
  return new (AircraftCanvasLayer as any)(opts);
}

export type AircraftCanvasLayer = L.Layer & {
  setData: (data: CanvasAircraft[], exclude: Set<string>, planeIcons: boolean) => void;
};

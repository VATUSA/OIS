import * as React from "react";

/**
 * Read design tokens (CSS custom properties from globals.css) from JS — for the places that can't
 * take `var(--x)`: deck.gl RGB arrays, chart SVG attributes, canvas. Everything else uses the
 * Tailwind utilities directly.
 */

export type Rgba = [number, number, number, number];

/** The current computed value of `--name` on <html> (empty string outside a browser). */
export function readToken(name: string): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();
}

/**
 * Parse a token colour — `#rgb`, `#rrggbb`, `rgb(r g b / a)` or `rgb(r, g, b)` / `rgba(…)` — into
 * 0–255 channels with alpha 0–255 (deck.gl's convention). Unparseable input yields opaque grey so a
 * missing token is visible rather than invisible.
 */
export function parseColor(value: string): Rgba {
  const v = value.trim();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(v);
  if (hex) {
    const h = hex[1].length === 3 ? [...hex[1]].map((c) => c + c).join("") : hex[1];
    const n = parseInt(h, 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 255];
  }
  const fn = /^rgba?\(([^)]+)\)$/i.exec(v);
  if (fn) {
    const parts = fn[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    if (parts.length >= 3 && parts.every((p) => Number.isFinite(p))) {
      const alpha = parts.length >= 4 ? Math.round(parts[3] * 255) : 255;
      return [parts[0], parts[1], parts[2], alpha];
    }
  }
  return [128, 128, 128, 255];
}

/** `readToken` + `parseColor`. */
export function tokenRgba(name: string): Rgba {
  return parseColor(readToken(name));
}

// The theme lives on <html class="dark">. ThemeProvider toggles it in an effect that runs *after*
// its children's effects, so re-reading on a React theme value would read stale tokens — observe the
// class attribute itself instead.
function subscribeThemeClass(onChange: () => void): () => void {
  if (typeof MutationObserver === "undefined") return () => {};
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
  return () => observer.disconnect();
}

const themeClass = () => (typeof document === "undefined" ? "" : document.documentElement.className);

/** Resolved token values keyed by name; re-reads when the theme class changes. */
export function useTokens<const N extends string>(names: readonly N[]): Record<N, string> {
  const cls = React.useSyncExternalStore(subscribeThemeClass, themeClass, () => "");
  const key = names.join("|");
  return React.useMemo(
    () => Object.fromEntries(names.map((n) => [n, readToken(n)])) as Record<N, string>,
    // `key` stands in for `names` so a fresh array literal each render doesn't re-read.
    [cls, key],
  );
}

/** Like `useTokens`, parsed to RGBA channel arrays (deck.gl). */
export function useTokenRgba<const N extends string>(names: readonly N[]): Record<N, Rgba> {
  const values = useTokens(names);
  return React.useMemo(
    () => Object.fromEntries(Object.entries(values).map(([k, v]) => [k, parseColor(v as string)])) as Record<N, Rgba>,
    [values],
  );
}

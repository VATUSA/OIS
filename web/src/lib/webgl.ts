/**
 * Whether the browser can create a WebGL2 context.
 *
 * deck.gl v9 renders exclusively through WebGL2, so the replay map needs it. All current iPhones/iPads
 * (iOS 15+) support WebGL2 — but iOS **Lockdown Mode** disables WebGL entirely, which paints the map as
 * a black rectangle on otherwise-capable hardware. We detect that here and show a fallback instead.
 */
export function webgl2Available(): boolean {
  try {
    const canvas = document.createElement("canvas");
    return canvas.getContext("webgl2") != null;
  } catch {
    return false;
  }
}

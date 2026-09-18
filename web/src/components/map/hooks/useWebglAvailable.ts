import {useCallback, useEffect, useState} from "react";

import {webgl2Available} from "@/lib/webgl";

export interface WebglStatus {
  /** Whether the map can be drawn: WebGL2 was there at mount and the context hasn't been lost since. */
  ok: boolean;
  /** Re-check WebGL2 and, if it's back, draw the map again. */
  retry: () => void;
}

/**
 * Whether WebGL2 is usable for the map. Availability is checked once on mount (deck.gl needs it;
 * iOS Lockdown Mode disables it), and a `webglcontextlost` anywhere on the page flips it off: once
 * the context is gone deck.gl can't relink its shaders, and the render error took the whole page
 * down with it rather than degrading (VATUSA/OIS#331).
 */
export function useWebglAvailable(): WebglStatus {
  const [ok, setOk] = useState(webgl2Available);

  useEffect(() => {
    // Capture phase, on window: the event is dispatched at deck.gl's own canvas, which it creates
    // internally and we hold no ref to. Capturing from the root reaches it wherever it sits, and
    // doesn't depend on whether the event bubbles.
    const onLost = () => setOk(false);
    window.addEventListener("webglcontextlost", onLost, true);
    return () => window.removeEventListener("webglcontextlost", onLost, true);
  }, []);

  const retry = useCallback(() => {
    // Re-probe rather than assuming: if WebGL is still off, the fallback simply stays put.
    // Flipping `ok` back on remounts deck.gl (the fallback replaced it), so it builds a fresh
    // context rather than reusing the dead canvas — no explicit remount key needed.
    setOk(webgl2Available());
  }, []);

  return { ok, retry };
}

import {useState} from "react";

import {webgl2Available} from "@/lib/webgl";

/** Whether WebGL2 is usable, checked once on mount (deck.gl needs it; iOS Lockdown Mode disables it). */
export function useWebglAvailable(): boolean {
  const [available] = useState(webgl2Available);
  return available;
}

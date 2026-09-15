import {useQuery} from "@tanstack/react-query";

import {ois} from "./api";

/** Cap on how far ahead the scrubber can project — matches the backend's own bound. */
export const MAX_PROJECTION_SEC = 90 * 60;

/**
 * The forward prediction scrubber's projected traffic (#226) — every airborne aircraft moved
 * `offsetSec` ahead along its own resolved route, using the same trajectory/ETA model as metering.
 * Disabled at `offsetSec <= 0` (the caller should fall back to live traffic instead — there's
 * nothing to project at T=0).
 */
export function usePredictedTraffic(offsetSec: number) {
  return useQuery({
    queryKey: ["predicted-traffic", offsetSec],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/traffic/projected", {
        params: { query: { offset_sec: offsetSec } },
      });
      if (error || !data) throw new Error("failed to load projected traffic");
      return data;
    },
    enabled: offsetSec > 0,
  });
}

import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type EventAvailability = components["schemas"]["EventAvailabilityBody"];

/** Who has indicated availability for an event (from the DCC thread 🟢/🟡/🔴 buttons). The query key
 *  is prefixed `event-availability` so the realtime nudge (`events.availability`) invalidates it. */
export function useEventAvailability(eventId: number) {
  return useQuery({
    queryKey: ["event-availability", eventId],
    queryFn: async (): Promise<EventAvailability[]> => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/availability", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load availability");
      return data;
    },
  });
}

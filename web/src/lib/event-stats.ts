import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";
import {useToast} from "@ois/ui";

export type EventCapture = components["schemas"]["EventCaptureBody"];
export type UpdateEventCapture = components["schemas"]["UpdateEventCaptureRequest"];
export type EventStats = components["schemas"]["EventStatsBody"];

/** Per-event stats-capture config + current capture status. */
export function useEventCapture(eventId: number) {
  return useQuery({
    queryKey: ["event-capture", eventId],
    enabled: Number.isFinite(eventId),
    queryFn: async (): Promise<EventCapture> => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/capture", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load capture config");
      return data;
    },
  });
}

export function useUpdateEventCapture(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpdateEventCapture): Promise<EventCapture> => {
      const { data, error, response } = await ois.PUT("/api/v1/events/{id}/capture", {
        params: { path: { id: eventId } },
        body,
      });
      if (response?.status === 403) throw new Error("forbidden");
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => {
      qc.setQueryData(["event-capture", eventId], data);
      qc.invalidateQueries({ queryKey: ["event-capture", eventId] });
    },
    onError: (e) =>
      toast.error(
        e.message === "forbidden"
          ? "You don’t have permission to change stats capture"
          : "Couldn’t save the capture setting",
      ),
  });
}

/** Generated debrief stats over an event's saved capture window. Only fetched when `enabled` (the
 * capture has been saved) — the debrief is a post-event artifact, not a live readout. */
export function useEventStats(eventId: number, enabled: boolean) {
  return useQuery({
    queryKey: ["event-stats", eventId],
    enabled: enabled && Number.isFinite(eventId),
    queryFn: async (): Promise<EventStats> => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/stats", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load event stats");
      return data;
    },
  });
}

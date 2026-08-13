import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type EventSummary = components["schemas"]["EventBody"];
export type Dcc = components["schemas"]["DccRequestBody"];
export type UpdateDcc = components["schemas"]["UpdateDccRequest"];

/** Upcoming (and in-progress) VATUSA events, soonest first. */
export function useUpcomingEvents() {
  return useQuery({
    queryKey: ["events"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events");
      if (error || !data) throw new Error("failed to load events");
      return data;
    },
  });
}

/** One event by VATUSA id. */
export function useEvent(id: number) {
  return useQuery({
    queryKey: ["event", id],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("failed to load event");
      return data;
    },
    enabled: Number.isFinite(id),
  });
}

/** DCC support state for one event (defaults to not_needed when unset). */
export function useDcc(eventId: number) {
  return useQuery({
    queryKey: ["event-dcc", eventId],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/dcc", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load DCC support");
      return data;
    },
    enabled: Number.isFinite(eventId),
  });
}

export function useUpdateDcc(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpdateDcc) => {
      const { data, error } = await ois.PUT("/api/v1/events/{id}/dcc", {
        params: { path: { id: eventId } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-dcc", eventId] });
      toast.success("DCC support updated");
    },
    onError: () => toast.error("Couldn’t update DCC support"),
  });
}

/** VATUSA's HTML/BBCode event blurb → plain text (safe to render, no markup). */
export function eventBodyText(body: string): string {
  return body
    .replace(/\[img\][^[]*\[\/img\]/gi, "") // drop BBCode images
    .replace(/\[[^\]]+\]/g, "") // drop remaining BBCode tags
    .replace(/<br\s*\/?>/gi, "\n") // <br> → newline
    .replace(/<[^>]+>/g, "") // drop HTML tags
    .replace(/&#0?39;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&nbsp;/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type EventSummary = components["schemas"]["EventBody"];

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

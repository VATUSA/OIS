import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type EventSummary = components["schemas"]["EventBody"];
export type Dcc = components["schemas"]["DccRequestBody"];
export type UpdateDcc = components["schemas"]["UpdateDccRequest"];
export type FacilitySupport = components["schemas"]["FacilitySupportBody"];
export type UpsertFacilitySupport =
  components["schemas"]["UpsertFacilitySupportRequest"];
export type AirportRate = components["schemas"]["AirportRateBody"];
export type UpsertAirportRate =
  components["schemas"]["UpsertAirportRateRequest"];

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

/** Facility support matrix for one event. */
export function useFacilitySupport(eventId: number) {
  return useQuery({
    queryKey: ["event-facilities", eventId],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/facilities", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load facility support");
      return data;
    },
    enabled: Number.isFinite(eventId),
  });
}

export function useUpsertFacilitySupport(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({
      facility,
      body,
    }: {
      facility: string;
      body: UpsertFacilitySupport;
    }) => {
      const { data, error } = await ois.PUT(
        "/api/v1/events/{id}/facilities/{facility}",
        { params: { path: { id: eventId, facility } }, body },
      );
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["event-facilities", eventId],
      });
    },
    onError: () => toast.error("Couldn’t save the facility"),
  });
}

export function useRemoveFacilitySupport(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (facility: string) => {
      const { error } = await ois.DELETE(
        "/api/v1/events/{id}/facilities/{facility}",
        { params: { path: { id: eventId, facility } } },
      );
      if (error) throw new Error("remove failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["event-facilities", eventId],
      });
      toast.success("Facility removed");
    },
    onError: () => toast.error("Couldn’t remove the facility"),
  });
}

/** Planned per-airport AAR/ADR for one event (each row carries an `editable` flag). */
export function useAirportRates(eventId: number) {
  return useQuery({
    queryKey: ["event-rates", eventId],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/rates", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load airport rates");
      return data;
    },
    enabled: Number.isFinite(eventId),
  });
}

export function useUpsertAirportRate(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({
      icao,
      body,
    }: {
      icao: string;
      body: UpsertAirportRate;
    }) => {
      const { data, error, response } = await ois.PUT(
        "/api/v1/events/{id}/rates/{icao}",
        { params: { path: { id: eventId, icao } }, body },
      );
      if (response?.status === 403) throw new Error("forbidden");
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-rates", eventId] });
    },
    onError: (e) =>
      toast.error(
        e.message === "forbidden"
          ? "You can only set rates for your own facility’s airports"
          : "Couldn’t save the rate",
      ),
  });
}

export function useRemoveAirportRate(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (icao: string) => {
      const { error, response } = await ois.DELETE(
        "/api/v1/events/{id}/rates/{icao}",
        { params: { path: { id: eventId, icao } } },
      );
      if (response?.status === 403) throw new Error("forbidden");
      if (error) throw new Error("remove failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-rates", eventId] });
      toast.success("Airport removed");
    },
    onError: (e) =>
      toast.error(
        e.message === "forbidden"
          ? "You can only remove your own facility’s airports"
          : "Couldn’t remove the airport",
      ),
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

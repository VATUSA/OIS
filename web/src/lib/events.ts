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
export type TmiPackage = components["schemas"]["TmiPackageBody"];
export type TmiPackageItem = components["schemas"]["TmiPackageItemBody"];

/**
 * The VATUSA website URL for editing an event. VATUSA gives us no URL — only the numeric id and the
 * facility (3-letter ARTCC) — so we construct it: /staff/facility/{facility}/events/{id}/edit.
 * Returns null when the event has no facility (can't build the staff path).
 */
export function vatusaEditUrl(event: Pick<EventSummary, "id" | "facility">): string | null {
  if (!event.facility) return null;
  return `https://vatusa.net/staff/facility/${event.facility}/events/${event.id}/edit`;
}

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

/** Enqueue a Discord coordination thread for the event. Needs `events.discord.publish`. */
export function usePublishEventDiscord(eventId: number) {
  const toast = useToast();
  return useMutation({
    mutationFn: async (): Promise<void> => {
      const { error, response } = await ois.POST(
        "/api/v1/events/{id}/discord/publish",
        { params: { path: { id: eventId } } },
      );
      if (response.status === 409) throw new Error("already");
      if (response.status === 400) throw new Error("unconfigured");
      if (error || !response.ok) throw new Error("failed");
    },
    onSuccess: () => toast.success("Event thread queued for Discord"),
    onError: (e) => {
      const msg =
        e instanceof Error && e.message === "already"
          ? "Already posted to Discord for this event."
          : e instanceof Error && e.message === "unconfigured"
            ? "Set an “events” channel in Admin → Discord first."
            : "Couldn’t post to Discord.";
      toast.error(msg);
    },
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
      const { data, error, response } = await ois.PUT(
        "/api/v1/events/{id}/facilities/{facility}",
        { params: { path: { id: eventId, facility } }, body },
      );
      if (response?.status === 403) throw new Error("forbidden");
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["event-facilities", eventId],
      });
    },
    onError: (e) =>
      toast.error(
        e.message === "forbidden"
          ? "You can only edit your own facility’s support"
          : "Couldn’t save the facility",
      ),
  });
}

export function useRemoveFacilitySupport(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (facility: string) => {
      const { error, response } = await ois.DELETE(
        "/api/v1/events/{id}/facilities/{facility}",
        { params: { path: { id: eventId, facility } } },
      );
      if (response?.status === 403) throw new Error("forbidden");
      if (error) throw new Error("remove failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["event-facilities", eventId],
      });
      toast.success("Facility support cleared");
    },
    onError: (e) =>
      toast.error(
        e.message === "forbidden"
          ? "You can only edit your own facility’s support"
          : "Couldn’t clear the facility",
      ),
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

/** TMI packages (draft bundles of programs/restrictions/ground stops) for an event. */
export function usePackages(eventId: number) {
  return useQuery({
    queryKey: ["event-packages", eventId],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/packages", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load packages");
      return data;
    },
    enabled: Number.isFinite(eventId),
  });
}

export function useCreatePackage(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (name: string) => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/packages", {
        params: { path: { id: eventId } },
        body: { name },
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
      toast.success("Package created");
    },
    onError: () => toast.error("Couldn’t create the package"),
  });
}

export function useDeletePackage(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (packageId: string) => {
      const { error } = await ois.DELETE(
        "/api/v1/events/{id}/packages/{package_id}",
        { params: { path: { id: eventId, package_id: packageId } } },
      );
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
      toast.success("Package deleted");
    },
    onError: () => toast.error("Couldn’t delete the package"),
  });
}

export function useAddPackageItem(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({
      packageId,
      kind,
      payload,
    }: {
      packageId: string;
      kind: string;
      payload: Record<string, unknown>;
    }) => {
      const { data, error } = await ois.POST(
        "/api/v1/events/{id}/packages/{package_id}/items",
        {
          params: { path: { id: eventId, package_id: packageId } },
          // payload is a free-form object typed as Object in the schema
          body: { kind, payload } as components["schemas"]["AddPackageItemRequest"],
        },
      );
      if (error || !data) throw new Error("add failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
    },
    onError: () => toast.error("Couldn’t add the item — check the fields"),
  });
}

export function useDeletePackageItem(eventId: number) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      packageId,
      itemId,
    }: {
      packageId: string;
      itemId: string;
    }) => {
      const { error } = await ois.DELETE(
        "/api/v1/events/{id}/packages/{package_id}/items/{item_id}",
        {
          params: {
            path: { id: eventId, package_id: packageId, item_id: itemId },
          },
        },
      );
      if (error) throw new Error("remove failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
    },
  });
}

export function useActivatePackage(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (packageId: string) => {
      const { data, error } = await ois.POST(
        "/api/v1/events/{id}/packages/{package_id}/activate",
        { params: { path: { id: eventId, package_id: packageId } } },
      );
      if (error || !data) throw new Error("activate failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
      // live TMU rows changed
      queryClient.invalidateQueries({ queryKey: ["tmu-programs"] });
      queryClient.invalidateQueries({ queryKey: ["tmis"] });
      queryClient.invalidateQueries({ queryKey: ["ground-stops"] });
      toast.success("Package activated — live in Operations");
    },
    onError: () => toast.error("Couldn’t activate the package"),
  });
}

export function useDeactivatePackage(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (packageId: string) => {
      const { data, error } = await ois.POST(
        "/api/v1/events/{id}/packages/{package_id}/deactivate",
        { params: { path: { id: eventId, package_id: packageId } } },
      );
      if (error || !data) throw new Error("deactivate failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-packages", eventId] });
      // live TMU rows were cancelled
      queryClient.invalidateQueries({ queryKey: ["tmu-programs"] });
      queryClient.invalidateQueries({ queryKey: ["tmis"] });
      queryClient.invalidateQueries({ queryKey: ["ground-stops"] });
      toast.success("Package deactivated — live TMIs cancelled");
    },
    onError: () => toast.error("Couldn’t deactivate the package"),
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

export type EventDebrief = components["schemas"]["EventDebriefBody"];

/** An event's free-text post-event debrief notes. */
export function useEventDebrief(eventId: number) {
  return useQuery({
    queryKey: ["event-debrief", eventId],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/debrief", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("failed to load debrief");
      return data;
    },
    enabled: Number.isFinite(eventId),
  });
}

export function useUpdateEventDebrief(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (notes: string) => {
      const { data, error } = await ois.PUT("/api/v1/events/{id}/debrief", {
        params: { path: { id: eventId } },
        body: { notes },
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["event-debrief", eventId] });
      toast.success("Debrief saved");
    },
    onError: () => toast.error("Couldn’t save the debrief"),
  });
}

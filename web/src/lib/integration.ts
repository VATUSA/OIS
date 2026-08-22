import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type DiscordConfig = components["schemas"]["DiscordConfigBody"];
export type DiscordMapEntry = components["schemas"]["DiscordMapEntry"];
export type UpsertDiscordConfig =
  components["schemas"]["UpsertDiscordConfigRequest"];
export type DiscordLink = components["schemas"]["DiscordLinkBody"];

const KEY = ["discord-config"] as const;
const LINK_KEY = ["discord-link"] as const;

/** The saved Discord guild config (channel/role/category maps). Needs `discord.config.read`. */
export function useDiscordConfig() {
  return useQuery({
    queryKey: KEY,
    queryFn: async (): Promise<DiscordConfig> => {
      const { data, error } = await ois.GET("/api/v1/integration/discord");
      if (error || !data) throw new Error("failed to load Discord config");
      return data;
    },
  });
}

/** Replace the Discord config and its logical-name maps. Needs `discord.config.update`. */
export function useUpdateDiscordConfig() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertDiscordConfig): Promise<DiscordConfig> => {
      const { data, error } = await ois.PUT("/api/v1/integration/discord", {
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => {
      qc.setQueryData(KEY, data);
      toast.success("Discord configuration saved");
    },
    onError: () => toast.error("Couldn’t save the Discord configuration"),
  });
}

// --- account linking (personal) ---

/** The current user's Discord account link. */
export function useDiscordLink() {
  return useQuery({
    queryKey: LINK_KEY,
    queryFn: async (): Promise<DiscordLink> => {
      const { data, error } = await ois.GET("/api/v1/me/discord");
      if (error || !data) throw new Error("failed to load Discord link");
      return data;
    },
  });
}

/** Redirect into the Discord OAuth flow; on return the browser lands back on this page. */
export function startDiscordLink() {
  const returnTo = `${window.location.origin}/profile`;
  window.location.href = `/api/v1/me/discord/link/start?return_to=${encodeURIComponent(returnTo)}`;
}

/** Remove the current user's Discord link. */
export function useUnlinkDiscord() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (): Promise<void> => {
      const { error } = await ois.DELETE("/api/v1/me/discord");
      if (error) throw new Error("unlink failed");
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: LINK_KEY });
      toast.success("Discord account unlinked");
    },
    onError: () => toast.error("Couldn’t unlink your Discord account"),
  });
}

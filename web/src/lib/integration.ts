import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type DiscordConfig = components["schemas"]["DiscordConfigBody"];
export type DiscordGuildConfig = components["schemas"]["DiscordGuildConfigBody"];
export type DiscordGuildConfigInput = components["schemas"]["DiscordGuildConfigInput"];
export type DiscordMapEntry = components["schemas"]["DiscordMapEntry"];
export type DiscordGuildSnapshot = components["schemas"]["DiscordGuildSnapshotBody"];
export type DiscordGuildChannel = components["schemas"]["DiscordGuildChannel"];
export type DiscordGuildRole = components["schemas"]["DiscordGuildRole"];
export type UpsertDiscordConfig =
  components["schemas"]["UpsertDiscordConfigRequest"];
export type DiscordLink = components["schemas"]["DiscordLinkBody"];

const KEY = ["discord-config"] as const;
const LINK_KEY = ["discord-link"] as const;

/** The Discord config: configured guilds + the bot's guild snapshot (for dropdowns). Needs
 *  `discord.config.read`. */
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

/** Replace the configured guilds + their logical-name maps. Needs `discord.config.update`. */
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

/** Ask the bot to re-pull its guild channels/roles (the "Refresh from Discord" button). The snapshot
 *  updates a moment later once the bot handles the job; refetch the config to see it. */
export function useRefreshDiscord() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (): Promise<void> => {
      const { error } = await ois.POST("/api/v1/integration/discord/refresh");
      if (error) throw new Error("refresh failed");
    },
    onSuccess: () => {
      toast.success("Refreshing from Discord — channels/roles update shortly");
      setTimeout(() => qc.invalidateQueries({ queryKey: KEY }), 2500);
    },
    onError: () => toast.error("Couldn’t reach the bot to refresh"),
  });
}

// --- Discord link (read-only; sourced from VATUSA) ---

/** The current user's Discord link, as synced from their VATUSA profile. */
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

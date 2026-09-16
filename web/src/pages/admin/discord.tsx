import {useEffect, useMemo, useState} from "react";
import {Plus, RefreshCw, Server, Trash2} from "lucide-react";
import {Button, Card, cn, EmptyState, Input, QueryState, Select, Textarea} from "@ois/ui";

import {usePageHeader} from "@/components/shell/page-meta";

import {
  type DiscordGuildSnapshot,
  type DiscordMapEntry,
  useDiscordConfig,
  useEventThreadTemplate,
  useRefreshDiscord,
  useUpdateDiscordConfig,
  useUpdateEventThreadTemplate,
} from "@/lib/integration";
import {useFacilities} from "@/lib/admin";

/** Logical channel names the backend resolves when enqueuing Discord jobs. Surfaced as hints so an
 *  admin knows which names actually drive a feature. Keep in sync with the handlers' constants. */
const KNOWN_CHANNELS: { name: string; hint: string }[] = [
  { name: "aceteam-requests", hint: "ACE coverage requests post + claim notifications" },
  { name: "tmu-advisories", hint: "Published TMIs (traffic management advisories)" },
  { name: "region-northeast", hint: "DCC threads for North East hosts (ZBW/ZDC/ZNY/ZOB)" },
  { name: "region-southeast", hint: "DCC threads for South East hosts (ZID/ZJX/ZMA/ZTL)" },
  { name: "region-southcentral", hint: "DCC threads for South Central hosts (ZAB/ZFW/ZHU/ZME)" },
  { name: "region-midwest", hint: "DCC threads for Midwest hosts (ZAU/ZDV/ZKC/ZMP)" },
  { name: "region-west", hint: "DCC threads for West hosts (ZAN/HCF/ZLA/ZLC/ZOA/ZSE)" },
  { name: "events", hint: "Fallback DCC-thread channel when a region has none" },
];

const SUBTITLE =
  "Map logical names to real channels/roles per server. Add both the DCC and VATUSA servers; each feature posts to whichever server has its channel mapped.";

/** Logical role names the backend resolves. */
const KNOWN_ROLES: { name: string; hint: string }[] = [
  { name: "ntmo", hint: "Pinged in DCC threads for NOM availability" },
  { name: "dcc-trainee", hint: "Pinged in DCC threads to shadow the event" },
];

type DraftGuild = {
  name: string;
  guild_id: string;
  channels: DiscordMapEntry[];
  roles: DiscordMapEntry[];
  facilities: string[];
};

type Opt = { id: string; label: string };

/** Channel snapshot → dropdown options (skip categories + threads, which aren't postable targets). */
function channelOptions(snap: DiscordGuildSnapshot | undefined): Opt[] {
  return (snap?.channels ?? [])
    .filter((c) => c.kind !== "category" && c.kind !== "thread")
    .map((c) => ({ id: c.id, label: (c.kind === "voice" ? "🔊 " : "#") + c.name }));
}

/** Role snapshot → dropdown options (skip @everyone, whose id equals the guild id). */
function roleOptions(snap: DiscordGuildSnapshot | undefined): Opt[] {
  return (snap?.roles ?? [])
    .filter((r) => r.id !== snap?.guild_id)
    .map((r) => ({ id: r.id, label: "@" + r.name }));
}

/** An editable list of logical-name → real-channel/role rows. The snowflake is chosen from a dropdown
 *  of the guild's actual channels/roles (from the bot snapshot) rather than typed by hand. */
function MapEditor({
  label,
  entries,
  options,
  onChange,
  suggestions,
}: {
  label: string;
  entries: DiscordMapEntry[];
  options: Opt[];
  onChange: (next: DiscordMapEntry[]) => void;
  suggestions: { name: string; hint: string }[];
}) {
  const update = (i: number, patch: Partial<DiscordMapEntry>) =>
    onChange(entries.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
  const remove = (i: number) => onChange(entries.filter((_, idx) => idx !== i));
  const add = (name = "") => onChange([...entries, { name, id: "" }]);
  const missing = suggestions.filter((s) => !entries.some((e) => e.name.trim() === s.name));

  return (
    <section className="flex flex-col gap-2.5 border-t border-line pt-5">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">{label}</h3>
        <Button type="button" variant="outline" size="sm" onClick={() => add()}>
          <Plus className="size-4" /> Add
        </Button>
      </div>

      {entries.length === 0 ? (
        <p className="text-sm text-ink-3">None mapped.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {entries.map((e, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                aria-label={`${label} logical name`}
                placeholder="logical-name"
                value={e.name}
                onChange={(ev) => update(i, { name: ev.target.value })}
                className="flex-1 font-mono"
              />
              <span className="text-ink-3">→</span>
              <Select
                aria-label={`${label} target`}
                value={e.id}
                onChange={(ev) => update(i, { id: ev.target.value })}
                wrapperClassName="min-w-0 flex-1"
              >
                <option value="">— pick —</option>
                {options.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.label}
                  </option>
                ))}
                {e.id && !options.some((o) => o.id === e.id) && (
                  <option value={e.id}>{e.id} (not in server)</option>
                )}
              </Select>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label="Remove"
                onClick={() => remove(i)}
              >
                <Trash2 className="size-4" />
              </Button>
            </div>
          ))}
        </div>
      )}

      {options.length === 0 && (
        <p className="text-xs text-ink-3">
          No {label.toLowerCase()} yet — set this server above and click “Refresh from Discord”.
        </p>
      )}

      {missing.length > 0 && (
        <div className="rounded-md border border-dashed border-line p-2.5 text-xs text-ink-2">
          <p className="mb-1.5 font-semibold">Used by features but not mapped yet:</p>
          <ul className="flex flex-col gap-1">
            {missing.map((s) => (
              <li key={s.name} className="flex items-center gap-2">
                <button
                  type="button"
                  className="rounded-full border border-line bg-chip px-2 py-0.5 font-mono text-ink hover:bg-panel-2"
                  onClick={() => add(s.name)}
                >
                  {s.name}
                </button>
                <span>— {s.hint}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function GuildCard({
  guild,
  snapshot,
  botGuilds,
  artccs,
  onChange,
  onRemove,
}: {
  guild: DraftGuild;
  snapshot: DiscordGuildSnapshot | undefined;
  botGuilds: DiscordGuildSnapshot[];
  artccs: string[];
  onChange: (next: DraftGuild) => void;
  onRemove: () => void;
}) {
  const set = (patch: Partial<DraftGuild>) => onChange({ ...guild, ...patch });
  const chOpts = channelOptions(snapshot);
  const roleOpts = roleOptions(snapshot);

  return (
    <Card className="flex flex-col gap-5 p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-xl font-bold">{guild.name.trim() || "New server"}</h2>
          <p className="text-sm text-ink-2">
            The Discord server this maps to. Pick it from the bot’s servers, then map channels + roles.
          </p>
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onRemove}>
          <Trash2 className="size-4" /> Remove
        </Button>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="font-semibold">Label</span>
          <Input
            placeholder="DCC"
            value={guild.name}
            onChange={(e) => set({ name: e.target.value })}
          />
        </label>
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="font-semibold">Server</span>
          {botGuilds.length > 0 ? (
            <Select
              value={guild.guild_id}
              onChange={(e) => {
                const gid = e.target.value;
                const g = botGuilds.find((b) => b.guild_id === gid);
                set({ guild_id: gid, name: guild.name.trim() || (g?.name ?? "") });
              }}
              wrapperClassName="w-full"
            >
              <option value="">— pick a server —</option>
              {botGuilds.map((g) => (
                <option key={g.guild_id} value={g.guild_id}>
                  {g.name}
                </option>
              ))}
              {guild.guild_id && !botGuilds.some((g) => g.guild_id === guild.guild_id) && (
                <option value={guild.guild_id}>{guild.guild_id} (bot not in server)</option>
              )}
            </Select>
          ) : (
            <Input
              placeholder="123456789012345678"
              value={guild.guild_id}
              inputMode="numeric"
              className="font-mono"
              onChange={(e) => set({ guild_id: e.target.value })}
            />
          )}
        </label>
      </div>

      {guild.guild_id && !snapshot && (
        <p className="rounded-md border border-line bg-warning-soft px-3 py-2 text-xs text-ink-2">
          The bot hasn’t reported this server’s channels/roles yet. Make sure the bot is in it, then
          click “Refresh from Discord”.
        </p>
      )}

      <section className="flex flex-col gap-1.5 border-t border-line pt-5">
        <h3 className="text-sm font-semibold">Facilities served</h3>
        <p className="text-xs text-ink-2">
          If another server maps the same logical channel/role name, the one whose facilities
          include the relevant ARTCC wins that name here — otherwise the first-configured server
          still wins, as before.
        </p>
        <div className="flex flex-wrap gap-1.5">
          {artccs.map((a) => {
            const active = guild.facilities.includes(a);
            return (
              <button
                key={a}
                type="button"
                aria-pressed={active}
                onClick={() =>
                  set({
                    facilities: active
                      ? guild.facilities.filter((f) => f !== a)
                      : [...guild.facilities, a],
                  })
                }
                className={cn(
                  "rounded-full border px-2.5 py-0.5 font-mono text-xs font-semibold transition-colors",
                  active
                    ? "border-brand/40 bg-brand-soft text-brand-ink"
                    : "border-line bg-panel-2 text-ink-2 hover:bg-chip hover:text-ink",
                )}
              >
                {a}
              </button>
            );
          })}
        </div>
      </section>

      <MapEditor
        label="Channels"
        entries={guild.channels}
        options={chOpts}
        onChange={(channels) => set({ channels })}
        suggestions={KNOWN_CHANNELS}
      />
      <MapEditor
        label="Roles"
        entries={guild.roles}
        options={roleOpts}
        onChange={(roles) => set({ roles })}
        suggestions={KNOWN_ROLES}
      />
    </Card>
  );
}

/** The event-thread message body template, edited independently of the guild list above (separate
 * endpoint, separate save action). */
function ThreadTemplateCard() {
  const query = useEventThreadTemplate();
  const save = useUpdateEventThreadTemplate();
  const [body, setBody] = useState("");

  const loaded = query.data;
  useEffect(() => {
    if (loaded) setBody(loaded.body);
  }, [loaded]);

  return (
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 className="text-xl font-bold">Event-thread message template</h2>
        <p className="text-sm text-ink-2">
          Posted at the top of every event planning thread. Placeholders:{" "}
          <code>{"{{title}}"}</code>, <code>{"{{date_line}}"}</code>,{" "}
          <code>{"{{facility_lines}}"}</code>, <code>{"{{ntmo_ping}}"}</code>,{" "}
          <code>{"{{dcc_ping}}"}</code>.
        </p>
      </div>
      <QueryState
        isLoading={!loaded && !query.isError}
        isError={query.isError}
        error="Couldn't load the template."
        onRetry={() => query.refetch()}
      >
        <div className="flex flex-col gap-3">
          <Textarea
            className="min-h-48 font-mono"
            value={body}
            onChange={(e) => setBody(e.target.value)}
          />
          <div className="flex justify-end">
            <Button
              size="sm"
              disabled={!body.trim() || save.isPending}
              onClick={() => save.mutate({ body })}
            >
              {save.isPending ? "Saving…" : "Save template"}
            </Button>
          </div>
        </div>
      </QueryState>
    </Card>
  );
}

export function AdminDiscord() {
  const query = useDiscordConfig();
  const save = useUpdateDiscordConfig();
  const refresh = useRefreshDiscord();
  const facilities = useFacilities();

  const [guilds, setGuilds] = useState<DraftGuild[]>([]);

  // Seed the form once the config loads (or resets after a save returns the canonical shape).
  const loaded = query.data;
  useEffect(() => {
    if (!loaded) return;
    setGuilds(
      loaded.guilds.map((g) => ({
        name: g.name,
        guild_id: g.guild_id,
        channels: g.channels,
        roles: g.roles,
        facilities: g.facilities,
      })),
    );
  }, [loaded]);

  const botGuilds = loaded?.available ?? [];
  const snapshotFor = (gid: string) => botGuilds.find((b) => b.guild_id === gid);
  const artccs = (facilities.data ?? [])
    .filter((f) => f.active)
    .map((f) => f.id)
    .sort();

  const clean = (entries: DiscordMapEntry[]) =>
    entries
      .map((e) => ({ name: e.name.trim(), id: e.id.trim() }))
      .filter((e) => e.name && e.id);

  const canSave = guilds.every((g) => g.name.trim() && g.guild_id.trim());

  const onSave = () =>
    save.mutate({
      guilds: guilds.map((g) => ({
        name: g.name.trim(),
        guild_id: g.guild_id.trim(),
        channels: clean(g.channels),
        facilities: g.facilities,
        roles: clean(g.roles),
      })),
    });

  const addGuild = () =>
    setGuilds((gs) => [...gs, { name: "", guild_id: "", channels: [], roles: [], facilities: [] }]);
  const updateGuild = (i: number, next: DraftGuild) =>
    setGuilds((gs) => gs.map((g, idx) => (idx === i ? next : g)));
  const removeGuild = (i: number) => setGuilds((gs) => gs.filter((_, idx) => idx !== i));

  const refreshing = refresh.isPending;
  const refreshNow = refresh.mutate;
  const actions = useMemo(
    () => (
      <Button type="button" variant="outline" onClick={() => refreshNow()} disabled={refreshing}>
        <RefreshCw className={cn("size-4", refreshing && "animate-spin")} />
        Refresh from Discord
      </Button>
    ),
    [refreshing, refreshNow],
  );
  usePageHeader({ subtitle: SUBTITLE, count: loaded ? loaded.guilds.length : null, actions });

  return (
    <div className="flex flex-col gap-6">
      {query.isError || !loaded ? (
        <Card>
          <QueryState
            isLoading={!loaded && !query.isError}
            isError={query.isError}
            error="Couldn't load the Discord configuration."
            onRetry={() => query.refetch()}
          />
        </Card>
      ) : (
        <>
          {guilds.length === 0 && (
            <Card>
              <EmptyState icon={Server} title="No servers configured yet">
                Add one to get started.
              </EmptyState>
            </Card>
          )}

          {guilds.map((g, i) => (
            <GuildCard
              key={i}
              guild={g}
              snapshot={g.guild_id ? snapshotFor(g.guild_id) : undefined}
              botGuilds={botGuilds}
              artccs={artccs}
              onChange={(next) => updateGuild(i, next)}
              onRemove={() => removeGuild(i)}
            />
          ))}

          <div className="flex items-center justify-between gap-3">
            <Button type="button" variant="outline" onClick={addGuild}>
              <Plus className="size-4" /> Add server
            </Button>
            <div className="flex items-center gap-3">
              {!canSave && (
                <span className="text-xs text-ink-3">
                  Every server needs a label and a selected server.
                </span>
              )}
              <Button onClick={onSave} disabled={!canSave || save.isPending}>
                {save.isPending ? "Saving…" : "Save configuration"}
              </Button>
            </div>
          </div>
        </>
      )}

      <ThreadTemplateCard />
    </div>
  );
}

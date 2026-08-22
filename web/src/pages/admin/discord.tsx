import {useEffect, useState} from "react";
import {Plus, Trash2} from "lucide-react";
import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
} from "@ois/ui";

import {
  type DiscordMapEntry,
  useDiscordConfig,
  useUpdateDiscordConfig,
} from "@/lib/integration";

/** Logical channel names the backend resolves when enqueuing Discord jobs. Surfaced as hints so an
 *  admin knows which names actually drive a feature. Keep in sync with the handlers' constants. */
const KNOWN_CHANNELS: { name: string; hint: string }[] = [
  { name: "aceteam-requests", hint: "ACE coverage requests post + claim notifications" },
  { name: "tmu-advisories", hint: "Published TMIs (traffic management advisories)" },
];

type Kind = "channels" | "roles" | "categories";

/** An editable list of logical-name → snowflake rows. */
function MapEditor({
  label,
  idLabel,
  entries,
  onChange,
  suggestions,
}: {
  label: string;
  idLabel: string;
  entries: DiscordMapEntry[];
  onChange: (next: DiscordMapEntry[]) => void;
  suggestions?: { name: string; hint: string }[];
}) {
  const update = (i: number, patch: Partial<DiscordMapEntry>) =>
    onChange(entries.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
  const remove = (i: number) => onChange(entries.filter((_, idx) => idx !== i));
  const add = (name = "") => onChange([...entries, { name, id: "" }]);

  const missing = (suggestions ?? []).filter(
    (s) => !entries.some((e) => e.name.trim() === s.name),
  );

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">{label}</h3>
        <Button type="button" variant="outline" size="sm" onClick={() => add()}>
          <Plus className="size-4" /> Add
        </Button>
      </div>

      {entries.length === 0 ? (
        <p className="text-sm text-muted-foreground">None mapped.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {entries.map((e, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                aria-label={`${label} name`}
                placeholder="logical-name"
                value={e.name}
                onChange={(ev) => update(i, { name: ev.target.value })}
                className="flex-1"
              />
              <Input
                aria-label={idLabel}
                placeholder={idLabel}
                value={e.id}
                inputMode="numeric"
                onChange={(ev) => update(i, { id: ev.target.value })}
                className="flex-1 font-mono"
              />
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

      {missing.length > 0 && (
        <div className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          <p className="mb-1 font-medium">Used by features but not mapped yet:</p>
          <ul className="flex flex-col gap-1">
            {missing.map((s) => (
              <li key={s.name} className="flex items-center gap-2">
                <button
                  type="button"
                  className="rounded bg-muted px-1.5 py-0.5 font-mono text-foreground hover:bg-accent"
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
    </div>
  );
}

export function AdminDiscord() {
  const query = useDiscordConfig();
  const save = useUpdateDiscordConfig();

  const [name, setName] = useState("");
  const [guildId, setGuildId] = useState("");
  const [channels, setChannels] = useState<DiscordMapEntry[]>([]);
  const [roles, setRoles] = useState<DiscordMapEntry[]>([]);
  const [categories, setCategories] = useState<DiscordMapEntry[]>([]);

  // Seed the form once the saved config loads (or resets after a save returns the canonical shape).
  const loaded = query.data;
  useEffect(() => {
    if (!loaded) return;
    setName(loaded.name ?? "");
    setGuildId(loaded.guild_id ?? "");
    setChannels(loaded.channels ?? []);
    setRoles(loaded.roles ?? []);
    setCategories(loaded.categories ?? []);
  }, [loaded]);

  const clean = (entries: DiscordMapEntry[]) =>
    entries
      .map((e) => ({ name: e.name.trim(), id: e.id.trim() }))
      .filter((e) => e.name && e.id);

  const canSave = name.trim().length > 0 && guildId.trim().length > 0;

  const onSave = () =>
    save.mutate({
      name: name.trim(),
      guild_id: guildId.trim(),
      channels: clean(channels),
      roles: clean(roles),
      categories: clean(categories),
    });

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          Discord Integration
        </h1>
        <p className="text-muted-foreground">
          Map logical names to Discord IDs. The backend enqueues jobs using these
          names; the bot resolves them to post in the right places.
        </p>
      </div>

      {query.isError ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            Couldn&apos;t load the Discord configuration.
          </CardContent>
        </Card>
      ) : !query.data ? (
        <Card>
          <CardContent className="py-10 text-center text-sm text-muted-foreground">
            Loading…
          </CardContent>
        </Card>
      ) : (
        <>
          <Card>
            <CardHeader>
              <CardTitle>Guild</CardTitle>
              <CardDescription>
                The server this integration posts to. Enable Developer Mode in
                Discord to copy IDs (right-click → Copy ID).
              </CardDescription>
            </CardHeader>
            <CardContent className="grid gap-4 sm:grid-cols-2">
              <label className="flex flex-col gap-1.5 text-sm">
                <span className="font-medium">Name</span>
                <Input
                  placeholder="VATUSA"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label className="flex flex-col gap-1.5 text-sm">
                <span className="font-medium">Guild ID</span>
                <Input
                  placeholder="123456789012345678"
                  value={guildId}
                  inputMode="numeric"
                  className="font-mono"
                  onChange={(e) => setGuildId(e.target.value)}
                />
              </label>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Mappings</CardTitle>
              <CardDescription>
                Logical name on the left, Discord ID on the right. Features
                reference channels by these names.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-8">
              <MapEditor
                label="Channels"
                idLabel="channel ID"
                entries={channels}
                onChange={setChannels}
                suggestions={KNOWN_CHANNELS}
              />
              <MapEditor
                label="Roles"
                idLabel="role ID"
                entries={roles}
                onChange={setRoles}
              />
              <MapEditor
                label="Categories"
                idLabel="category ID"
                entries={categories}
                onChange={setCategories}
              />
            </CardContent>
          </Card>

          <div className="flex items-center justify-end gap-3">
            {!canSave && (
              <span className="text-xs text-muted-foreground">
                Name and Guild ID are required.
              </span>
            )}
            <Button onClick={onSave} disabled={!canSave || save.isPending}>
              {save.isPending ? "Saving…" : "Save configuration"}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

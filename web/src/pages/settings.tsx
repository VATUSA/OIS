import {Card, CardContent, CardHeader, CardTitle, Switch} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {SETTINGS, type SettingDef} from "@/lib/settings";
import {useSetting} from "@/lib/settings";

const SELECT_CLASS =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** One settings row — its own component so the `useSetting` hook is called once per setting. */
function SettingRow({ def }: { def: SettingDef }) {
  const { value, setValue } = useSetting(def.key, def.control.default);
  return (
    <div className="flex items-center justify-between gap-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-medium">{def.label}</div>
        {def.description && (
          <p className="mt-0.5 text-xs leading-snug text-muted-foreground">{def.description}</p>
        )}
      </div>
      <div className="shrink-0">
        {def.control.kind === "toggle" ? (
          <Switch
            checked={value as boolean}
            onCheckedChange={(v) => setValue(v)}
            aria-label={def.label}
          />
        ) : (
          <select
            className={SELECT_CLASS}
            value={value as string}
            onChange={(e) => setValue(e.target.value)}
            aria-label={def.label}
          >
            {def.control.options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        )}
      </div>
    </div>
  );
}

export function SettingsPage() {
  const { data: me, isLoading } = useMe();

  if (isLoading) {
    return <p className="py-10 text-center text-sm text-muted-foreground">Loading…</p>;
  }
  if (!me) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">Sign in to change your settings.</p>
    );
  }

  // Group settings by `group`, preserving first-seen order.
  const groups: { name: string; defs: SettingDef[] }[] = [];
  for (const def of SETTINGS) {
    let g = groups.find((x) => x.name === def.group);
    if (!g) {
      g = { name: def.group, defs: [] };
      groups.push(g);
    }
    g.defs.push(def);
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground">Preferences saved to your account.</p>
      </div>
      {groups.map((g) => (
        <Card key={g.name}>
          <CardHeader>
            <CardTitle className="text-base">{g.name}</CardTitle>
          </CardHeader>
          <CardContent className="divide-y pt-0">
            {g.defs.map((def) => (
              <SettingRow key={def.key} def={def} />
            ))}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

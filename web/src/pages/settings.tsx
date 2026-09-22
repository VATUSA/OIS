import {Card, EmptyState, HotkeyInput, QueryState, Select, Switch} from "@ois/ui";
import {LogIn} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {resumeHotkeys, suspendHotkeys} from "@/lib/hotkeys";
import {useMe} from "@/lib/auth";
import {SETTINGS, type SettingDef} from "@/lib/settings";
import {useSetting} from "@/lib/settings";

/** One settings row — its own component so the `useSetting` hook is called once per setting. */
function SettingRow({ def }: { def: SettingDef }) {
  const { value, setValue } = useSetting(def.key, def.control.default);
  return (
    <div className="flex items-center justify-between gap-4 border-b border-line-soft py-3 last:border-b-0">
      <div className="min-w-0">
        <div className="text-sm font-semibold text-ink">{def.label}</div>
        {def.description && <p className="mt-0.5 text-xs leading-snug text-ink-2">{def.description}</p>}
      </div>
      <div className="shrink-0">
        {def.control.kind === "toggle" ? (
          <Switch checked={value as boolean} onCheckedChange={(v) => setValue(v)} aria-label={def.label} />
        ) : def.control.kind === "hotkey" ? (
          <HotkeyInput
            value={value as string}
            placeholder={def.control.placeholder}
            onChange={(v) => setValue(v)}
            // Hand the live shortcuts back while recording: a registered combination is swallowed
            // by the OS, so rebinding one to another action would otherwise be impossible.
            onCaptureChange={(capturing) => {
              if (capturing) void suspendHotkeys();
              else resumeHotkeys();
            }}
            aria-label={def.label}
          />
        ) : (
          <Select value={value as string} onChange={(e) => setValue(e.target.value)} aria-label={def.label}>
            {def.control.options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </Select>
        )}
      </div>
    </div>
  );
}

export function SettingsPage() {
  const { data: me, isLoading } = useMe();
  usePageHeader({ subtitle: "Preferences saved to your account." });

  if (isLoading) return <QueryState isLoading />;
  if (!me) return <EmptyState icon={LogIn}>Sign in to change your settings.</EmptyState>;

  // Group settings by `group`, preserving first-seen order.
  const groups: { name: string; defs: SettingDef[] }[] = [];
  for (const def of SETTINGS.filter((d) => d.available?.() ?? true)) {
    let g = groups.find((x) => x.name === def.group);
    if (!g) {
      g = { name: def.group, defs: [] };
      groups.push(g);
    }
    g.defs.push(def);
  }

  return (
    <div className="flex w-full max-w-3xl flex-col gap-4">
      {groups.map((g) => (
        <Card key={g.name} className="flex flex-col gap-1 p-5">
          <h2 className="text-xl font-bold text-ink">{g.name}</h2>
          <div className="flex flex-col">
            {g.defs.map((def) => (
              <SettingRow key={def.key} def={def} />
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}

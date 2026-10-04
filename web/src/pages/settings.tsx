import type * as React from "react";
import {Card, EmptyState, HotkeyInput, QueryState, Select, Switch} from "@ois/ui";
import {LogIn} from "lucide-react";

import {SendDiagnosticsButton} from "@/components/send-diagnostics";
import {SoundPreviewButton} from "@/components/settings/sound-preview";
import {usePageHeader} from "@/components/shell/page-meta";
import {resumeHotkeys, suspendHotkeys} from "@/lib/hotkeys";
import {useMe} from "@/lib/auth";
import {SETTINGS, type SettingDef} from "@/lib/settings";
import {useSetting} from "@/lib/settings";
import {can} from "@/lib/platform";
import {useLoginItem} from "@/lib/tray";

/** One settings row — its own component so the `useSetting` hook is called once per setting. */
function SettingRow({ def }: { def: SettingDef }) {
  // Not an account setting — read from and written to this computer (see `LoginItemControl`).
  if (def.control.kind === "loginItem") return <LoginItemRow def={def} />;
  return <AccountSettingRow def={def} />;
}

function LoginItemRow({ def }: { def: SettingDef }) {
  const { enabled, setEnabled } = useLoginItem();
  return (
    <SettingRowFrame def={def}>
      <Switch
        checked={enabled ?? false}
        disabled={enabled === undefined}
        onCheckedChange={setEnabled}
        aria-label={def.label}
      />
    </SettingRowFrame>
  );
}

function SettingRowFrame({ def, children }: { def: SettingDef; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-line-soft py-3 last:border-b-0">
      <div className="min-w-0">
        <div className="text-sm font-semibold text-ink">{def.label}</div>
        {def.description && <p className="mt-0.5 text-xs leading-snug text-ink-2">{def.description}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function AccountSettingRow({ def }: { def: SettingDef }) {
  const control = def.control;
  const { value, setValue } = useSetting(def.key, "default" in control ? control.default : false);
  return (
    <SettingRowFrame def={def}>
      {control.kind === "hotkey" ? (
        <HotkeyInput
          value={value as string}
          placeholder={control.placeholder}
          onChange={(v) => setValue(v)}
          // Hand the live shortcuts back while recording: a registered combination is swallowed
          // by the OS, so rebinding one to another action would otherwise be impossible.
          onCaptureChange={(capturing) => {
            if (capturing) void suspendHotkeys();
            else void resumeHotkeys();
          }}
          aria-label={def.label}
        />
      ) : control.kind === "select" ? (
        <Select value={value as string} onChange={(e) => setValue(e.target.value)} aria-label={def.label}>
          {control.options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
      ) : control.kind === "soundToggle" ? (
        <div className="flex items-center gap-1">
          {/* Audition the tone without switching the category on — see `SoundPreviewButton`. */}
          <SoundPreviewButton category={control.category} />
          <Switch checked={value as boolean} onCheckedChange={(v) => setValue(v)} aria-label={def.label} />
        </div>
      ) : (
        <Switch checked={value as boolean} onCheckedChange={(v) => setValue(v)} aria-label={def.label} />
      )}
    </SettingRowFrame>
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
      {can("diagnostics") && (
        <Card className="flex flex-col gap-3 p-5">
          <h2 className="text-xl font-bold text-ink">Diagnostics</h2>
          <p className="text-sm text-ink-2">
            If something isn’t working, send this app’s recent logs to OIS staff so they can look into
            it. Nothing is sent unless you choose to.
          </p>
          <div>
            <SendDiagnosticsButton />
          </div>
        </Card>
      )}
    </div>
  );
}

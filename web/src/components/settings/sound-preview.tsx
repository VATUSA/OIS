import * as React from "react";
import {Button} from "@ois/ui";
import {Volume2} from "lucide-react";

import type {NotifyCategory} from "@/lib/desktop-notify";
import {useSetting} from "@/lib/settings";
import {previewAlertSound, type AlertSource} from "@/lib/sounds";

/** What the user needs to know: whose file they just heard. */
const ANSWER: Record<Exclude<AlertSource, "none">, string> = {
  replacement: "Your file",
  bundled: "Bundled default",
};

/**
 * Plays one alert category on demand and says which file was heard (#404).
 *
 * The tones are replaceable, but a replacement that can't be read falls back to the bundled default
 * in silence — so a wrong folder, a wrong name, the wrong case on Linux (`eventreminders.wav`) or a
 * codec the webview rejects were all indistinguishable from success. Tones are decoded once per
 * process (#353), so an alert keeps playing the copy it decoded first; a preview always reads the
 * file as it is on disk now, and alerts in this window pick that up from then on.
 *
 * The answer goes beside the button rather than into a toast: the question is about the row you are
 * looking at.
 */
export function SoundPreviewButton({category}: {category: NotifyCategory}) {
  const {value: volume} = useSetting<string>(`sounds.${category}.volume`, "normal");
  const [played, setPlayed] = React.useState<AlertSource | undefined>();
  const [busy, setBusy] = React.useState(false);

  return (
    <div className="flex items-center gap-2">
      <span aria-live="polite" className="text-xs text-ink-2">
        {played === undefined ? null : played === "none" ? "Couldn't play it" : ANSWER[played]}
      </span>
      <Button
        size="icon"
        variant="ghost"
        aria-label={`Play the ${category} alert sound`}
        title="Hear this alert"
        className="size-7 text-ink-3 hover:text-ink"
        disabled={busy}
        onClick={() => {
          // Cleared so a repeat press with the same answer is still announced, and visibly ran.
          setPlayed(undefined);
          setBusy(true);
          void previewAlertSound(category, volume)
            .then(setPlayed)
            .finally(() => setBusy(false));
        }}
      >
        <Volume2 className="size-4" />
      </Button>
    </div>
  );
}

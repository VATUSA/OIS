import * as React from "react";
import {Download} from "lucide-react";
import {Button, useToast} from "@ois/ui";

import {installUpdate, useDesktopUpdate} from "@/lib/desktop-update";

/**
 * Tells the user a verified update is waiting, and lets them take it when they choose.
 *
 * Renders nothing at all on the web build and on the desktop until a package has been downloaded
 * *and* its signature verified — so if this is on screen, restarting is safe.
 *
 * Deliberately a passive strip rather than a modal: the person looking at it may be running
 * traffic, and an update is never urgent enough to interrupt that.
 */
export function UpdateBanner() {
  const status = useDesktopUpdate();
  const toast = useToast();
  const [installing, setInstalling] = React.useState(false);

  if (status.state !== "ready") return null;

  // Applying can still fail — the package may be unreadable, or the relaunch may be refused. A
  // bare `void installUpdate()` made that a button that silently did nothing.
  const restart = () => {
    setInstalling(true);
    installUpdate(status.staged).catch((error: unknown) => {
      setInstalling(false);
      toast.error("Update failed", {
        description:
          error instanceof Error ? error.message : "The update could not be applied.",
      });
    });
  };

  return (
    <div className="flex items-center gap-3 border-b border-line bg-panel-2 px-4 py-2 text-sm">
      <Download className="size-4 shrink-0 text-brand" aria-hidden />
      <span className="text-ink-2">
        OIS <span className="font-mono text-ink">{status.version}</span> is ready to install.
      </span>
      <Button size="sm" className="ml-auto" onClick={restart} disabled={installing}>
        {installing ? "Restarting..." : "Restart to update"}
      </Button>
    </div>
  );
}

import {Download} from "lucide-react";
import {Button} from "@ois/ui";

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

  if (status.state !== "ready") return null;

  return (
    <div className="flex items-center gap-3 border-b border-line bg-panel-2 px-4 py-2 text-sm">
      <Download className="size-4 shrink-0 text-brand" aria-hidden />
      <span className="text-ink-2">
        OIS <span className="font-mono text-ink">{status.version}</span> is ready to install.
      </span>
      <Button size="sm" className="ml-auto" onClick={() => void installUpdate()}>
        Restart to update
      </Button>
    </div>
  );
}

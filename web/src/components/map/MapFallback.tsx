import {TriangleAlert} from "lucide-react";
import {Button} from "@ois/ui";

/**
 * Shown in place of the map when WebGL2 isn't usable — either it was unavailable at mount (iOS
 * Lockdown Mode, a blocklisted GPU) or the context was lost mid-session, which leaves deck.gl
 * unable to relink its shaders. Without this the page used to white-screen (VATUSA/OIS#331).
 *
 * `onRetry` re-checks WebGL and remounts the map; it's omitted when there's nothing to retry.
 */
export function MapFallback({ onRetry }: { onRetry?: () => void }) {
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-3 px-6 py-16 text-center">
      <TriangleAlert className="size-8 text-ink-3" />
      <div className="text-lg font-semibold">Map can&apos;t be drawn here</div>
      <p className="max-w-md text-sm text-ink-2">
        This map needs WebGL, and this browser has it turned off or couldn&apos;t keep it running.
        Opening OIS in another browser usually fixes it.
      </p>
      <p className="max-w-md text-sm text-ink-2">
        On iPhone and iPad it&apos;s almost always{" "}
        <span className="font-semibold text-ink">Lockdown Mode</span>: tap{" "}
        <span className="font-semibold text-ink">ᴀA</span> in Safari&apos;s address bar →{" "}
        <span className="font-semibold text-ink">Website Settings</span> →{" "}
        <span className="font-semibold text-ink">Lockdown Mode → Off</span>, then reload.
      </p>
      {onRetry && (
        <Button variant="outline" onClick={onRetry}>
          Try again
        </Button>
      )}
    </div>
  );
}

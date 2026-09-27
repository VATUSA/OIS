import {DesktopNotifiers} from "@/components/desktop-notifiers";
import {NotificationClicks} from "@/components/notification-clicks";
import {PrimaryWindowOnly} from "@/components/primary-window-only";

/**
 * The features that must run once per *app*, not once per window (#350).
 *
 * A group rather than three siblings in `router.tsx` so that "these are wrapped in
 * {@link PrimaryWindowOnly}" is a property of something a test can mount. It was not: removing the
 * wrapper from the root layout left the whole suite green, because the wrapper was tested on its own
 * and nothing tested its use — which is how one ground stop firing a notification per open window,
 * and one click making every window raise and navigate itself, reached review twice.
 *
 * Anything added here must be an OS-level side effect. Per-window things — the realtime socket, the
 * query cache, in-app toasts and alerts — stay in the root layout; see the note on
 * {@link PrimaryWindowOnly} for why the socket in particular cannot be shared.
 */
export function PrimaryWindowFeatures() {
  return (
    <PrimaryWindowOnly>
      <NotificationClicks />
      <DesktopNotifiers />
    </PrimaryWindowOnly>
  );
}

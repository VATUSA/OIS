import {useEffect} from "react";
import {useQueryClient} from "@tanstack/react-query";

import {useMe} from "@/lib/auth";
import {connectRealtime} from "@/lib/realtime";

/**
 * Opens the realtime websocket for signed-in users so operational surfaces (IDST, FCA ladder,
 * departures, TMU boards) update instantly on release/GDP/TMI changes. No-op when signed out (public
 * pages just poll); the socket is additive over the REST API.
 */
export function RealtimeProvider({ children }: { children: React.ReactNode }) {
  const qc = useQueryClient();
  const authed = !!useMe().data;
  useEffect(() => {
    if (!authed) return;
    return connectRealtime(qc);
  }, [qc, authed]);
  return <>{children}</>;
}

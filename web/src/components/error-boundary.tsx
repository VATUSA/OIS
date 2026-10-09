import * as React from "react";
import {Button} from "@ois/ui";

import {SendDiagnosticsButton} from "@/components/send-diagnostics";
import {describe, log} from "@/lib/logger";

/** Logs a render crash, so it leaves a line in the log rather than only a white screen (#629). */
export function logRenderCrash(error: unknown, componentStack?: string | null) {
  log("error", `render crash: ${describe(error)}${componentStack ? `\n${componentStack}` : ""}`, "react");
}

/**
 * What a crashed window shows instead of a white screen: a way back, and — on the desktop — a way to
 * report it.
 */
export function CrashScreen() {
  return (
    <div className="flex min-h-dvh flex-col items-center justify-center gap-4 bg-ground px-6 text-center text-ink">
      <p className="text-xl font-bold">Something went wrong</p>
      <p className="max-w-md text-sm text-ink-2">
        This window hit an error it couldn’t recover from. Reloading usually fixes it.
      </p>
      <div className="flex gap-2">
        <Button onClick={() => window.location.reload()}>Reload</Button>
        <SendDiagnosticsButton />
      </div>
    </div>
  );
}

/** The last line of defence above the router, whose own `errorComponent` catches route errors. */
export class ErrorBoundary extends React.Component<{children: React.ReactNode}, {crashed: boolean}> {
  state = {crashed: false};

  static getDerivedStateFromError() {
    return {crashed: true};
  }

  componentDidCatch(error: unknown, info: React.ErrorInfo) {
    logRenderCrash(error, info.componentStack);
  }

  render() {
    return this.state.crashed ? <CrashScreen /> : this.props.children;
  }
}

/** The router's `errorComponent`: logs the route error once, then shows the crash screen. */
export function RouteErrorScreen({error}: {error: unknown}) {
  React.useEffect(() => logRenderCrash(error), [error]);
  return <CrashScreen />;
}

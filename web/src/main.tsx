import React from "react";
import ReactDOM from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {RouterProvider} from "@tanstack/react-router";
import {DialogProvider, ThemeProvider, ToastProvider, TooltipProvider} from "@ois/ui";

import {router} from "./router";
import {desktopRefresh} from "./lib/desktop-auth";
import {isTauri} from "./lib/platform";
import {RealtimeProvider} from "./components/realtime-provider";
import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/600.css";
import "./index.css";

const queryClient = new QueryClient();

// Desktop only: rotate the keychain-stored session on launch, which both proves it is still valid
// and pushes its expiry out, so an app that's opened regularly never makes the user sign in again
// (#346).
//
// Awaited, because rotation DELETES the old session row server-side the moment it succeeds. Any
// request that left while the new token was still in flight would carry one that is already dead,
// and `fetchMe` turns a 401 into a cached "signed out" for a full minute — which presents as the
// app randomly forgetting you on launch. Holding first paint for one loopback round trip is the
// cheaper trade. A failure just means we start signed out, which is what the UI would show anyway.
async function bootstrap() {
  if (isTauri()) {
    await desktopRefresh().catch(() => undefined);
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider defaultTheme="dark">
      <ToastProvider>
        <DialogProvider>
          <TooltipProvider>
            <QueryClientProvider client={queryClient}>
              <RealtimeProvider>
                <RouterProvider router={router} />
              </RealtimeProvider>
            </QueryClientProvider>
          </TooltipProvider>
        </DialogProvider>
      </ToastProvider>
    </ThemeProvider>
  </React.StrictMode>,
  );
}

void bootstrap();

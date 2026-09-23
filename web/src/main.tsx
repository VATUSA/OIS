import React from "react";
import ReactDOM from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {RouterProvider} from "@tanstack/react-router";
import {DialogProvider, ThemeProvider, ToastProvider, TooltipProvider} from "@ois/ui";

import {router} from "./router";
import {refreshBeforeLaunch} from "./lib/desktop-auth";
import {restoreWindows} from "./lib/popout";
import {isMainWindow, isTauri} from "./lib/platform";
import {RealtimeProvider} from "./components/realtime-provider";
import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/600.css";
import "./index.css";

const queryClient = new QueryClient();

// Desktop only: reopen the route windows that were open last time, each at the position it was
// left (#350). Guarded inside to the main window — otherwise every restored window would restore
// the whole set again as it booted. Not awaited; a window failing to reopen must not delay paint.
if (isTauri()) void restoreWindows();

// Desktop only: rotate the keychain-stored session on launch, which both proves it is still valid
// and pushes its expiry out, so an app that's opened regularly never makes the user sign in again
// (#346).
//
// Awaited, because rotation DELETES the old session row server-side the moment it succeeds. Any
// request that left while the new token was still in flight would carry one that is already dead,
// and `fetchMe` turns a 401 into a cached "signed out" for a full minute — which presents as the
// app randomly forgetting you on launch. So first paint waits for the rotation — for at most
// `LAUNCH_REFRESH_BUDGET_MS`, since the API is remote and a blackholed host would otherwise leave a
// blank window until the OS gives up. A failure just means we start signed out.
//
// Guarded to the MAIN window, for the same reason `restoreWindows()` above is: every Tauri webview
// runs this module. Unguarded, each pop-out (#349) and route window (#350) rotated the session on
// open — and rotation DELETES the token it is handed, leaving the windows that were already open
// holding a dead one, with no 401 recovery path. Opening a second window signed the first out.
async function bootstrap() {
  if (isTauri() && (await isMainWindow())) {
    await refreshBeforeLaunch();
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

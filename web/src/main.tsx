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
// (#346). Deliberately not awaited — the stored token stays valid meanwhile, so there is no reason
// to hold up first paint, and a failure here just means the app starts signed out.
if (isTauri()) void desktopRefresh();

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

import React from "react";
import ReactDOM from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {RouterProvider} from "@tanstack/react-router";
import {DialogProvider, ThemeProvider, ToastProvider} from "@ois/ui";

import {router} from "./router";
import {RealtimeProvider} from "./components/realtime-provider";
import "./index.css";

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider defaultTheme="system">
      <ToastProvider>
        <DialogProvider>
          <QueryClientProvider client={queryClient}>
            <RealtimeProvider>
              <RouterProvider router={router} />
            </RealtimeProvider>
          </QueryClientProvider>
        </DialogProvider>
      </ToastProvider>
    </ThemeProvider>
  </React.StrictMode>,
);

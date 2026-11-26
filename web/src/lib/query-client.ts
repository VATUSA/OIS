import {QueryClient} from "@tanstack/react-query";

/**
 * The app's one query cache. `main.tsx` provides it to React; route guards (`route-guard.ts`) read
 * `["me"]` from it in `beforeLoad`, which runs outside React and has no hook to reach it through.
 */
export const queryClient = new QueryClient();

import type {Stage} from "@/lib/stage";

/** This build's release stage, from the repo-root `STAGE` file via the Vite `define` (#841). */
export const BUILD_STAGE: Stage = __APP_STAGE__;

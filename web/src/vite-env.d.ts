/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OIS_API_URL?: string;
}
interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/** Full app version, e.g. "1.0.1-a1b2c3d" (base VERSION + commit) — injected by Vite `define`. */
declare const __APP_VERSION__: string;

/** Release stage from the repo-root STAGE file, validated at build time (#841). Read it through `@/lib/build-stage`. */
declare const __APP_STAGE__: import("./lib/stage").Stage;

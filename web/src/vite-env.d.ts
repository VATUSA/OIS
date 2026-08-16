/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OIS_API_URL?: string;
}
interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/** App version (web/package.json) and short build commit — injected by Vite `define`. */
declare const __APP_VERSION__: string;
declare const __APP_SHA__: string;

// Lint gate for the JS workspaces (`pnpm lint` → each workspace's `eslint src`). Type-checking stays
// with `tsc`; this catches what it can't: hook-order mistakes and dead code.
import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: ["**/dist/**", "**/node_modules/**", "packages/api-client/src/generated/**", "docs-site/**"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      // Blocking as of #329: every omission that existed when this was a warning has been either
      // fixed or annotated with an `eslint-disable-next-line` saying why the dependency is left out,
      // so a *new* one is an unreviewed stale closure and should fail the gate. Deliberately keeping
      // a dependency out is still allowed — say why on the disable line.
      "react-hooks/exhaustive-deps": "error",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
    },
  },
  {
    // The desktop app renders the web bundle inside a Tauri webview, so `@tauri-apps/api` is a
    // real dependency of `web` — but a *static* import anywhere pulls it into the main bundle that
    // every browser downloads. `web/src/lib/platform.ts` reaches it through a dynamic `import()`
    // behind an `isTauri()` guard, which keeps it in a chunk the web build never loads.
    //
    // This rule only matches static imports, so platform.ts needs no exception — and deliberately
    // doesn't get one, so that adding a static import *there* is caught too. Go through
    // `invokeDesktop()` rather than importing Tauri directly (#345).
    // `packages/**` is in scope too: @ois/ui is consumed as raw source (its `exports` point at
    // src/index.ts), so it compiles into the very browser bundle this rule protects — and shared
    // shell chrome, where a tray or pop-out control would live, belongs there per DESIGN.md.
    files: ["web/**/*.{ts,tsx}", "packages/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/*"],
              message:
                "Import Tauri only via web/src/lib/platform.ts (invokeDesktop) — a static import here would bloat the web bundle.",
            },
          ],
        },
      ],
    },
  },
);

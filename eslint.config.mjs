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
      // Reported, not blocking: changing a dependency list changes when an effect re-runs, so
      // existing omissions are fixed deliberately rather than to satisfy the gate (#306).
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
    },
  },
);

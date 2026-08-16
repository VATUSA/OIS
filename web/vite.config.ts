import {defineConfig} from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import {execSync} from "node:child_process";
import {readFileSync} from "node:fs";
import path from "node:path";

// Programmer-controlled base version, single source of truth for the whole repo.
function baseVersion(): string {
  try {
    return readFileSync(path.resolve(import.meta.dirname, "..", "VERSION"), "utf8").trim();
  } catch {
    return "0.0.0";
  }
}

function gitSha(): string {
  try {
    return execSync("git rev-parse --short HEAD", { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim();
  } catch {
    return "";
  }
}

// Full app version shown in the footer, e.g. "1.0.1-a1b2c3d". In the image build CI passes the
// finished string as OIS_VERSION (`.git` is dockerignored); locally we build it from the VERSION
// file plus the current commit.
function appVersion(): string {
  if (process.env.OIS_VERSION) return process.env.OIS_VERSION;
  const base = baseVersion();
  const sha = gitSha();
  return sha ? `${base}-${sha}` : base;
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  define: {
    __APP_VERSION__: JSON.stringify(appVersion()),
  },
  // Bind to 127.0.0.1 so the web origin matches the API host (127.0.0.1:3000) —
  // otherwise the session cookie (SameSite=Lax) is treated as cross-site and dropped.
  server: { host: "127.0.0.1", port: 5173, strictPort: true },
});

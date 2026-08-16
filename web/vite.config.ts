import {defineConfig} from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import {execSync} from "node:child_process";
import {readFileSync} from "node:fs";
import path from "node:path";

const pkg = JSON.parse(
  readFileSync(path.resolve(import.meta.dirname, "package.json"), "utf8"),
) as { version?: string };

// Short commit for the footer build stamp. `.git` is dockerignored, so in the image build the SHA
// comes from the OIS_GIT_SHA build arg (see deploy/web.Dockerfile + build-images.yml); locally we
// fall back to reading git directly. Empty string when neither is available.
function gitSha(): string {
  const fromEnv = process.env.OIS_GIT_SHA;
  if (fromEnv) return fromEnv.slice(0, 7);
  try {
    return execSync("git rev-parse --short HEAD", { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim();
  } catch {
    return "";
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version ?? "0.0.0"),
    __APP_SHA__: JSON.stringify(gitSha()),
  },
  // Bind to 127.0.0.1 so the web origin matches the API host (127.0.0.1:3000) —
  // otherwise the session cookie (SameSite=Lax) is treated as cross-site and dropped.
  server: { host: "127.0.0.1", port: 5173, strictPort: true },
});

import {defineConfig} from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  // Bind to 127.0.0.1 so the web origin matches the API host (127.0.0.1:3000) —
  // otherwise the session cookie (SameSite=Lax) is treated as cross-site and dropped.
  server: { host: "127.0.0.1", port: 5173, strictPort: true },
});

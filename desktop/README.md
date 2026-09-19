# `desktop/` — the OIS desktop app

A Tauri shell around the **existing `web/` SPA**. It is not a second frontend: `tauri.conf.json`
points `frontendDist` at `web`'s Vite output, so `@ois/ui`, `@ois/api-client` and all of `web/src`
ship unchanged and the desktop app is literally the same app.

```bash
just desktop        # dev: starts the web dev server, hot-reloads it into the webview
just desktop-build  # bundle for the host platform (builds web/dist first)
```

## Layout

| Path | What |
| --- | --- |
| `src-tauri/` | The `ois-desktop` crate — a member of the root Cargo workspace |
| `src-tauri/tauri.conf.json` | Window, bundle and dev/build wiring |
| `src-tauri/capabilities/` | Tauri v2 permissions, scoped per window |
| `package.json` | Holds `@tauri-apps/cli` only |

## Things that look like oversights but aren't

- **`package.json` defines no `build`/`lint`/`typecheck`/`test` script.** Turbo skips packages that
  lack a task, which is exactly what we want — a `build` script here would make the repo-wide
  `pnpm build` try to bundle a native app, and `pnpm typecheck` would fail on a package with no
  TypeScript. Keep desktop tasks in the `justfile`.
- **The API base URL needs no desktop special-case.** `web/src/lib/api.ts` already resolves an
  absolute base, so the SPA reaches the backend from `tauri://localhost` unchanged. Auth is the part
  that does differ — the desktop app uses a keychain-stored token rather than the session cookie
  (#346).
- **The backend must allow the webview's origin.** It isn't an http host: `tauri://localhost` on
  macOS/Linux, `http://tauri.localhost` on Windows. Both are in `.env.example`'s
  `CORS_ALLOWED_ORIGINS`; a deployment that drops them gets a desktop app whose every API call is
  blocked by CORS, which looks exactly like the server being down. `normalize_origin` in
  `backend/src/config.rs` passes non-http schemes through untouched, so no code change is needed.
- **`cargo check` works without `web/dist`.** Debug builds use `devUrl`; only release builds embed
  the frontend, and `beforeBuildCommand` produces it. A fresh clone's `just ci` is therefore fine.
- **No `version` in `tauri.conf.json`.** The bundle inherits the crate version; release versioning
  arrives with distribution + auto-update (#347).

## What isn't here yet

The shell is deliberately bare — one window, no commands, no plugins. Desktop behaviour lands in
later issues, each adding its own plugins and capability entries: the platform capability layer and
IPC conventions (#345), keychain auth (#346), distribution and signed auto-update (#347), then the
feature set (#348–#354). See the epic, #343.

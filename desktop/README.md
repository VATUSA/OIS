# `desktop/` — the OIS desktop app

A Tauri shell around the **existing `web/` SPA**. It is not a second frontend: `tauri.conf.json`
points `frontendDist` at `web`'s Vite output, so `@ois/ui`, `@ois/api-client` and all of `web/src`
ship unchanged and the desktop app is literally the same app.

```bash
just desktop        # dev: starts the web dev server, hot-reloads it into the webview
just desktop-build  # bundle for the host platform (builds web/dist first)
```

## Linux prerequisites

Tauri's Linux backend needs GTK/WebKit headers. Because `ois-desktop` is a workspace member, this
is a prerequisite for the repo-wide `just ci` too, not just for desktop work — a backend-only change
fails the gate without it. macOS and Windows use the OS webview and need nothing extra.

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
```

The container images are unaffected: `deploy/backend.Dockerfile` and `deploy/discord.Dockerfile`
cook only their own crate (`cargo chef cook -p ois-backend` / `-p ois-discord`), so neither pulls the
desktop dependency tree into a `rust:1-bookworm` builder that has no GTK.

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
- **The API base is baked in at bundle time, and that still needs solving.** `web/src/lib/api.ts`
  resolves an *absolute* base, so nothing breaks merely because the page is served from
  `tauri://localhost`. But the web app learns its base at **runtime**: `deploy/40-ois-config.sh`
  writes `window.__OIS_API_URL__` into `config.js` at container start. A Tauri bundle has no
  container start — `web/public/config.js` is a comment-only placeholder — so the SPA falls through
  to `VITE_OIS_API_URL`, and then to the `http://127.0.0.1:3000` default. A plain `just desktop-build`
  therefore ships an app that only ever talks to the build machine's own localhost. Dev is unaffected
  (`devUrl` is the web dev server, which already points at the local backend). Choosing how a
  released desktop build learns its API base — a build-time `VITE_OIS_API_URL`, or a desktop-side
  runtime config — belongs to distribution, **#347**; until then, set `VITE_OIS_API_URL` explicitly
  when you bundle anything you intend to hand to someone else. Auth differs too: the desktop app uses
  a keychain-stored token rather than the session cookie (#346).
- **The backend must allow the webview's origin.** It isn't an http host: `tauri://localhost` on
  macOS/Linux, `http://tauri.localhost` on Windows. Both are in `.env.example`'s
  `CORS_ALLOWED_ORIGINS`; a deployment that drops them gets a desktop app whose every API call is
  blocked by CORS, which looks exactly like the server being down. `normalize_origin` in
  `backend/src/config.rs` passes non-http schemes through untouched, so no code change is needed —
  pinned by `config::tests::passes_the_desktop_webview_origins_through_untouched`, because tightening
  that function to http(s)-only would silently drop the desktop origin.
  Note the same list is reused by `validate_return_to` (`backend/src/handlers/auth.rs`) as the OAuth
  redirect allowlist, so these entries widen that too: `http://tauri.localhost` becomes a valid
  post-login redirect target for ordinary browser logins, while `tauri://localhost` is *rejected*
  there by its `http`/`https` scheme guard — something #346 has to resolve before it can return an
  OAuth flow into the webview.
- **`cargo check` works without `web/dist`.** Embedding the frontend happens in the `tauri build`
  CLI, which runs `beforeBuildCommand` to produce `web/dist` first; a bare `cargo check`/`cargo build`
  never embeds and tolerates the directory being absent, in debug *and* release. So a fresh clone can
  `just ci` without building the web app — **on macOS and Windows**. On Linux it first needs the
  GTK/WebKit dev packages (see below), because `just check` is `cargo check --workspace` and the
  workspace now contains `ois-desktop`.
- **No `version` in `tauri.conf.json`.** The bundle inherits the crate version; release versioning
  arrives with distribution + auto-update (#347).

## Content Security Policy

`tauri.conf.json` sets a CSP for bundled builds (#346). It matters because the webview can call the
app's commands, and `get_token` hands back the 30-day keychain token: without a policy, one injected
script could read it and post it anywhere.

| Directive | Allows | Why |
| --- | --- | --- |
| `script-src 'self'` | the bundle's own scripts | Tauri hashes the inline pre-paint theme script in `index.html` into this automatically; nothing else inline may run. |
| `style-src 'self' 'unsafe-inline'` | inline styles | maplibre and React set element styles at runtime. |
| `img-src`, `connect-src` → `https://*.cartocdn.com` | basemap style, tiles, sprites, glyphs | The map's only third-party origin. |
| `worker-src 'self' blob:` | blob workers | maplibre spawns its tile workers from blobs. |
| `connect-src ipc: http://ipc.localhost` | Tauri IPC | How `invoke` reaches the commands. |
| `connect-src http://127.0.0.1:3000 ws://127.0.0.1:3000` | the API, REST + realtime (`/api/v1/ws`) | The default API base. **A build pointed at another API must add that origin, `http(s)` and `ws(s)`.** Choosing the shipped API base is #347's, which owes that entry. |

`devCsp` is `null`: `just desktop` loads the Vite dev server, whose HMR and module preamble a strict
policy would block. The policy is enforced only on the bundled app.

## What isn't here yet

Keychain auth (#346) adds the app's first commands: `begin_login`, `get_token`, `store_token`,
`delete_token`. The rest of desktop behaviour lands in later issues, each adding its own plugins and
capability entries: distribution and signed auto-update (#347), then the feature set (#348–#354).
See the epic, #343.

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
- **The API base is baked in at bundle time.** `web/src/lib/api.ts`
  resolves an *absolute* base, so nothing breaks merely because the page is served from
  `tauri://localhost`. But the web app learns its base at **runtime**: `deploy/40-ois-config.sh`
  writes `window.__OIS_API_URL__` into `config.js` at container start. A Tauri bundle has no
  container start — `web/public/config.js` is a comment-only placeholder — so the SPA falls through
  to `VITE_OIS_API_URL`, and then to the `http://127.0.0.1:3000` default. A plain `just desktop-build`
  therefore ships an app that only ever talks to the build machine's own localhost. Dev is unaffected
  (`devUrl` is the web dev server, which already points at the local backend). **Releases** take the
  base from the repo variable `OIS_DESKTOP_API_URL`: the release job runs
  `desktop/scripts/pin-release-api.py`, which exports it as `VITE_OIS_API_URL` for the web build and
  adds the origin to the CSP (below), and fails the release if the variable is unset, not `https://`,
  carries a path, or points at loopback (#347). A local bundle you intend to hand to someone else
  needs the same: `VITE_OIS_API_URL=https://… just desktop-build`, plus the CSP entry. Auth differs
  too: the desktop app uses a keychain-stored token rather than the session cookie (#346).
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
  there by its `http`/`https` scheme guard. Sign-in (#346) sidesteps that: it returns to a loopback
  listener listed in `OAUTH_RETURN_TO_ORIGINS`, not to the webview.
- **`cargo check` works without `web/dist`.** Embedding the frontend happens in the `tauri build`
  CLI, which runs `beforeBuildCommand` to produce `web/dist` first; a bare `cargo check`/`cargo build`
  never embeds and tolerates the directory being absent, in debug *and* release. So a fresh clone can
  `just ci` without building the web app — **on macOS and Windows**. On Linux it first needs the
  GTK/WebKit dev packages (see above), because `just check` is `cargo check --workspace` and the
  workspace now contains `ois-desktop`.
- **`tauri.conf.json` carries an explicit `version`, and the release job rewrites it.** It has to:
  the bundle used to inherit the crate's hardcoded `0.1.0`, so every release shipped `0.1.0`, the
  manifest advertised `0.1.0`, and an installed app asked "is `0.1.0` newer than `0.1.0`?" — the
  update channel never fired for anyone. The checked-in value tracks `VERSION` so a local
  `just desktop-build` is honest; the release job replaces it with the tag and **fails the build if
  the tag and `VERSION` disagree**, because a version that is silently wrong costs you the whole
  update channel with nothing to notice.
- **`bundle.createUpdaterArtifacts` is `true`, and must stay that way.** It defaults to `false`, and
  without it `tauri build` produces installers but no `.tar.gz`/`.sig` pair — so the release has
  nothing for `latest.json` to point at and the updater has nothing to verify. The signing key being
  present does not help: there is simply nothing to sign.

## Releasing, and the signing keys

A `v*` tag cuts the GitHub Release and attaches the installers plus `latest.json` — the feed the
app's updater polls (`.github/workflows/release.yml`). Backend, web and desktop therefore all ship
on one version number: the job derives the desktop version from the tag and refuses to build if it
disagrees with `VERSION`, so bump `VERSION` in the same change that you tag.

The three platform legs run one at a time (`max-parallel: 1`) because each publishes the same
`latest.json` asset to the same release; in parallel the last upload wins and the manifest can end
up missing a platform, with every job still green.

**Before the first release, someone has to generate the updater keypair.** It is not in the repo,
and deliberately was not generated by tooling — a signing key that has passed through anything other
than your own machine is not a key you can trust:

```bash
pnpm --filter desktop exec tauri signer generate -w ~/.tauri/ois-updater.key
```

That prints a public key and writes a private one. Then:

1. Put the **public** key in `desktop/src-tauri/tauri.conf.json` under `plugins.updater.pubkey`,
   replacing `REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY`. It is public by design — it belongs in git.
2. Put the **private** key and its password in repo secrets as `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit it.

The release job refuses to run while the placeholder is still there, so it is not possible to
publish an update the installed app would reject.

### Secrets the release uses

| Secret | Required | For |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` + `_PASSWORD` | **Yes** | Signs each update package; the app verifies it against the pubkey above and refuses a mismatch |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | No | macOS code signing + notarisation |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | No | Windows code signing |

**Repository variable** (Settings → Variables, not a secret — it is baked into a public binary):

| Variable | Required | For |
| --- | --- | --- |
| `OIS_DESKTOP_API_URL` | **Yes** | The production API origin the shipped app talks to, e.g. `https://api.example.org` (no path, no trailing slash). Added to the CSP `connect-src` as `https://` and `wss://`. The release fails without it. |

Without the Apple/Windows certificates the build still succeeds, but the OS warns on first launch.
The updater signature above is what gates an update **applying** — a different thing from OS signing.

Unsigned on macOS used to cost more than a warning, and the history is worth keeping (#535).

⚠️ Add each Apple env line in `.github/workflows/release.yml` **only once its secret is set**. A
*present-but-blank* `APPLE_CERTIFICATE` makes the bundler attempt signing and fail on `security
import`, which is what broke the v0.1.1 build.

## Where the session token is stored

**This is not the same place on every platform, and macOS is the odd one out.**

| Platform | Store |
| --- | --- |
| macOS | `~/Library/Application Support/net.vatusa.ois/session`, mode `0600` |
| Windows | Credential Manager, via `keyring` |
| Linux | Secret Service, via `keyring` |

### Why macOS does not use the keychain

It used to, and it asked the user for their **login password** over and over:

1. The token was stored with `keyring::Entry`, which on macOS writes to the **legacy login keychain**.
2. macOS attaches a **per-item ACL** to that entry naming the application allowed to read it, and
   identifies that application by its **code-signing identity**.
3. An unsigned build has no stable identity — at best an ad-hoc signature, whose cdhash differs on
   **every build**.
4. So the app asking to read the token was not, as far as macOS was concerned, the app that wrote it.
   It fell back to asking the user to authorise with their login password.
5. And because `createUpdaterArtifacts: true` replaces the `.app` on every release, the identity
   rotated for **existing** users too — never only a fresh-install problem.

A Developer ID certificate fixes that by giving the app a stable identity. **We don't have one**, so
the token goes somewhere with no ACL instead: a file. No ACL, no prompt, signed or not.

Windows and Linux keep their credential stores. Neither behaves this way and neither prompts anybody,
so moving them would trade real OS protection for nothing.

### What protects the file

**Its permissions, and nothing else.** `0600` means only the user's own account can read it. A process
already running as that user can read it, which the keychain would have prevented — that is the real
cost of this trade, and it is stated rather than dressed up. Encrypting the file would require a key
that also lives on this disk, which is obfuscation, not a control.

What bounds the exposure is that the token **rotates on every launch** (`web/src/lib/desktop-auth.ts`),
so a copy lifted from disk has a short life.

### Upgrading from a build that used the keychain

The first read after updating finds no file, so it reads the keychain once, writes what it finds to the
file, and deletes the keychain entry. **That is the last keychain prompt a user will ever see**, and
only users upgrading get even that. If it is dismissed or fails, the result is "signed out" and signing
in again writes the file — the keychain is not consulted a second time.

The file is **truncated, not deleted**, on sign-out: its existence is what records that the migration
already happened. Deleting it would send the next launch back through the keychain and reintroduce the
prompt.

## Replacing the alert sounds

Each alert category ships with a default `.wav`, but a facility can use its own without a rebuild.
Drop a file of the same name into the app's data folder:

```
macOS    ~/Library/Application Support/net.vatusa.ois/sounds/
Windows  %APPDATA%\net.vatusa.ois\sounds\
Linux    ~/.local/share/net.vatusa.ois/sounds/
```

The names are the alert categories: `restrictions.wav`, `releases.wav`, `metering.wav`,
`access.wav`, `eventReminders.wav`. A file that is missing or won't play falls back to the bundled
default, so a bad replacement degrades to the standard sound rather than to silence — silence is
indistinguishable from a broken feature.

**A replacement is picked up at the next launch.** Tones are fetched and decoded once and then kept
for the life of the process, so dropping a file in while the app is running does not change the sound
until it restarts. (An earlier note here claimed otherwise; it did not account for the decode cache,
nor for the webview caching an unchanged URL.)

Sounds play through **Web Audio** — fetched whole, decoded, then played through a gain node — rather
than through an `Audio` element. A release build serves the bundle over Tauri's `tauri://` protocol,
which has no byte-range handling, and a webview's media stack asks for media with Range requests, so
an `<audio>` source could fail in the shipped build while working in dev over Vite's http. A plain
`fetch` needs no Range support, so both take the same path.

The CSP (see above) has no `media-src`, so `connect-src` carries `asset:` and
`http://asset.localhost` — without them `default-src 'self'` blocks a replacement tone outright.

The asset-protocol scope in `tauri.conf.json` is deliberately `$APPDATA/sounds/*` and nothing
wider: the webview can read a replaced alert sound and no other file on the machine.

## Content Security Policy

`tauri.conf.json` sets a CSP for bundled builds (#346). It matters because the webview can call the
app's commands, and `get_token` hands back the 30-day keychain token: without a policy, one injected
script could read it and post it anywhere.

| Directive | Allows | Why |
| --- | --- | --- |
| `script-src 'self'` | the bundle's own scripts | Tauri hashes the inline pre-paint theme script in `index.html` into this automatically; nothing else inline may run. |
| `style-src 'self' 'unsafe-inline'` | inline styles | maplibre and React set element styles at runtime. |
| `img-src`, `connect-src` → `https://*.cartocdn.com` | basemap style, tiles, sprites, glyphs | The map's only third-party origin. |
| `img-src blob:` | event banners | Banners are third-party URLs and organisers use any host, so no allowlist can cover them (#429). They are fetched through the API by `EventBanner` and handed over as an object URL — nothing is added to `img-src` for them. |
| `worker-src 'self' blob:` | blob workers | maplibre spawns its tile workers from blobs. |
| `connect-src ipc: http://ipc.localhost` | Tauri IPC | How `invoke` reaches the commands. |
| `connect-src http://127.0.0.1:3000 ws://127.0.0.1:3000` | the API, REST + realtime (`/api/v1/ws`) | The default API base. **A build pointed at another API must add that origin, `http(s)` and `ws(s)`.** Release builds get it from `OIS_DESKTOP_API_URL` via `desktop/scripts/pin-release-api.py` (#347). |

`devCsp` is `null`: `just desktop` loads the Vite dev server, whose HMR and module preamble a strict
policy would block. The policy is enforced only on the bundled app.

**That is why a CSP mistake reaches users.** Dev has no policy, so a blocked resource looks perfectly
fine until the bundle ships — which is how event banners were broken for the whole of #429's life.
Anything that loads a remote resource must be checked in a *bundled* build, or kept off remote
origins entirely the way banners now are.

## What isn't here yet

Keychain auth (#346) adds the app's first commands: `begin_login`, `get_token`, `store_token`,
`delete_token`; signed auto-update (#347) adds the updater and process plugins. The rest of desktop
behaviour lands in later issues, each adding its own plugins and capability entries: the feature
set (#348–#354).
See the epic, #343.

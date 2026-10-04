//! Desktop authentication: the OS keychain, and the loopback half of the VATSIM OAuth flow (#346).
//!
//! The desktop app cannot carry the website's session cookie — a Tauri webview has no browser
//! origin — so it authenticates with a session token sent as `Authorization: Bearer ois_dsk_…`.
//! That token lives in the real OS keychain (macOS Keychain, Windows Credential Manager, Linux
//! Secret Service), which is also what makes the session survive a restart.
//!
//! Sign-in follows RFC 8252 (OAuth for native apps): we open the **system browser**, not an
//! in-app one, so the app never observes the user's VATSIM credentials. The browser lands on a
//! loopback URL we are listening on, carrying a single-use code, which the frontend trades for the
//! real token. The token itself never travels through a URL.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use uuid::Uuid;

/// Keychain coordinates. The service name is the app's bundle identifier so the entry is
/// identifiable in Keychain Access / Credential Manager.
const KEYCHAIN_SERVICE: &str = "net.vatusa.ois";
const KEYCHAIN_USER: &str = "desktop-session";

/// The loopback redirect target. Fixed rather than ephemeral because the backend's `return_to`
/// allowlist is static env config (`CORS_ALLOWED_ORIGINS`), so a port chosen at runtime could not
/// be authorised. Bound only on 127.0.0.1, so nothing off-machine can reach it.
const LOOPBACK_ADDR: &str = "127.0.0.1:8765";
pub const LOOPBACK_REDIRECT: &str = "http://127.0.0.1:8765/callback";

/// How long we hold the listener open waiting for the user to finish signing in.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// How long one accepted connection gets to deliver its request line. Generous for loopback, and
/// short enough that a connection which opens and says nothing can't stall the accept loop.
const PER_CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a new sign-in waits for a superseded one to let go of the port.
///
/// Must exceed [`PER_CONNECTION_READ_TIMEOUT`]: a superseded attempt only notices it has been
/// replaced between connections, so one sitting in a slow read has to be allowed to finish it first.
const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(6);

/// Which sign-in attempt owns the loopback listener.
///
/// Only one can: the redirect target is a fixed port ([`LOOPBACK_ADDR`]). Without this a second
/// attempt simply failed to bind, and because the first holds the port for the rest of
/// [`LOGIN_TIMEOUT`], a user who abandoned a sign-in in the browser was wedged for five minutes
/// (#428). Each new attempt claims a higher generation, and the previous one stands down.
static LOGIN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Binds the loopback listener, waiting out a sign-in that is on its way down.
///
/// A superseded attempt drops its listener within one poll of noticing, so the retry is normally a
/// single sleep. `AddrInUse` past the deadline means something else on the machine holds the port —
/// reported as before, since no amount of waiting will free it.
fn bind_with_takeover(addr: &str) -> Result<TcpListener, String> {
    bind_until(addr, Instant::now() + TAKEOVER_TIMEOUT)
}

/// The retry itself, with the deadline handed in so a test can exercise the give-up path without
/// sitting out [`TAKEOVER_TIMEOUT`] — the version that inlined the deadline could only be tested by
/// re-implementing it, which is not a test of anything (#428 review).
fn bind_until(addr: &str, deadline: Instant) -> Result<TcpListener, String> {
    loop {
        match TcpListener::bind(addr) {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("could not listen on {addr}: {e}")),
        }
    }
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| format!("keychain unavailable: {e}"))
}

/// The token, cached for the life of the process.
///
/// Every Tauri webview loads its own copy of the frontend, so each pop-out (#349) and each restored
/// route window has its own in-memory cache and used to make its own `get_token` call — N windows
/// meant N keychain reads. On macOS every read is ACL-gated and can put up a login-password prompt,
/// so that multiplied the prompts (#535). Caching here makes it one read per process however many
/// windows ask. [`store_token`] and [`delete_token`] keep it in step, since both already know the
/// value they wrote.
///
/// The outer `Option` is "have we read yet?"; the inner one is "is there a token?", so a genuine
/// signed-out answer is cached rather than re-read.
static CACHED_TOKEN: Mutex<Option<Option<String>>> = Mutex::new(None);

/// Takes the cache lock, recovering a poisoned one.
///
/// A panic while holding this lock would otherwise make every later keychain call fail; the cached
/// value is a plain `Option<String>` that cannot be left half-written, so there is no invariant for
/// poisoning to protect.
fn lock<T>(cache: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The cached token, reading through `read` only on the first call.
///
/// Takes the cache by reference rather than reaching for [`CACHED_TOKEN`] so a test can exercise it
/// with a local cache and a counting reader — no keychain, and no dependence on test ordering.
///
/// A failing read is **not** cached: the loop it would otherwise cause is stopped on the frontend
/// (`web/src/lib/desktop-token.ts`), and poisoning the process cache would mean a transient keychain
/// error could never recover without a restart.
fn cached_or_read(
    cache: &Mutex<Option<Option<String>>>,
    read: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    let mut guard = lock(cache);
    if let Some(token) = guard.as_ref() {
        return Ok(token.clone());
    }
    let token = read()?;
    *guard = Some(token.clone());
    Ok(token)
}

/// Reads the keychain itself, with a missing entry meaning signed out rather than an error.
fn read_keychain() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read the keychain: {e}")),
    }
}

/// Where the session token lives, which is not the same place on every platform.
///
/// **macOS does not use the keychain** (#535). Its login keychain attaches a per-item ACL naming the
/// application allowed to read the entry, and identifies that application by its code-signing
/// identity. Unsigned builds have no stable identity — the cdhash differs on every build — so macOS
/// stops recognising the app that wrote the token and falls back to asking the user for their login
/// password. A Developer ID certificate would fix that; not having one, the token goes somewhere with
/// no ACL instead, where no prompt is possible.
///
/// **Windows and Linux keep their credential stores.** Neither has this behaviour and neither prompts
/// anybody today, so moving them to a file would trade real OS protection for nothing.
///
/// On macOS the file's permissions are the protection. Encrypting it would need a key that also lives
/// on this disk, which is obfuscation rather than a control, so it is written plainly and
/// `desktop/README.md` says what it is. The exposure is bounded by the token rotating on every launch
/// (`web/src/lib/desktop-auth.ts`), so a copy taken from disk has a short life.
#[cfg(target_os = "macos")]
mod store {
    use std::{
        fs,
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    use tauri::{AppHandle, Manager};

    /// Owner read/write only. The whole security story for this file.
    const FILE_MODE: u32 = 0o600;

    /// What the session file says.
    ///
    /// `Missing` is distinct from `SignedOut` on purpose: a missing file means this install has not
    /// been migrated off the keychain yet, while an empty one means migration happened and nobody is
    /// signed in. Collapsing the two would send every signed-out launch back through the keychain and
    /// prompt every time — the bug this is closing.
    #[derive(Debug, PartialEq)]
    pub(super) enum Stored {
        Missing,
        SignedOut,
        Token(String),
    }

    pub(super) fn read_file(path: &Path) -> Result<Stored, String> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                let token = contents.trim();
                Ok(if token.is_empty() {
                    Stored::SignedOut
                } else {
                    Stored::Token(token.to_string())
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::Missing),
            // Anything else — unreadable, a directory, bad encoding — reads as signed out rather
            // than taking the app down. The user can sign in again, which rewrites the file.
            Err(e) => {
                log::warn!("session file unreadable, treating as signed out: {e}");
                Ok(Stored::SignedOut)
            }
        }
    }

    /// Writes the token, creating the file 0600. An empty `token` is the signed-out marker.
    pub(super) fn write_file(path: &Path, token: &str) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(path)
            .map_err(|e| format!("could not open the session file: {e}"))?;
        file.write_all(token.as_bytes())
            .map_err(|e| format!("could not write the session file: {e}"))?;
        // `mode` only applies when *creating*, so an existing file keeps whatever it had. Set it
        // again so a file from an earlier, looser write is tightened rather than trusted.
        fs::set_permissions(path, fs::Permissions::from_mode(FILE_MODE))
            .map_err(|e| format!("could not set permissions on the session file: {e}"))?;
        Ok(())
    }

    /// Reads the file, migrating off the keychain the first time.
    ///
    /// The migration read is the **one and only** keychain prompt a user will ever see, and only
    /// users upgrading from a build that stored there get even that. Whatever happens, the file is
    /// written — so migration is attempted exactly once and a signed-out user is never sent back
    /// through the keychain.
    ///
    /// The keychain entry is deleted only when the read actually produced a token: deleting is
    /// ACL-gated too, so attempting it after a failed read risks a *second* prompt, which is the one
    /// thing this change exists to prevent. An entry left behind is inert, because the file's
    /// existence means it is never read again.
    pub(super) fn read_or_migrate(
        path: &Path,
        read_keychain: impl FnOnce() -> Result<Option<String>, String>,
        delete_keychain: impl FnOnce() -> Result<(), String>,
    ) -> Result<Option<String>, String> {
        match read_file(path)? {
            Stored::Token(token) => Ok(Some(token)),
            Stored::SignedOut => Ok(None),
            Stored::Missing => {
                // A dismissed or broken read is signed out. The write below still marks migration
                // done, so this costs one prompt in total, not one per launch.
                let migrated = read_keychain().unwrap_or_else(|e| {
                    log::warn!(
                        "keychain read during migration failed, treating as signed out: {e}"
                    );
                    None
                });
                write_file(path, migrated.as_deref().unwrap_or(""))?;
                if migrated.is_some() {
                    // Harmless if left behind (the file now wins), but worth knowing about.
                    if let Err(e) = delete_keychain() {
                        log::warn!("could not remove the migrated keychain entry: {e}");
                    }
                }
                Ok(migrated)
            }
        }
    }

    fn session_path(app: &AppHandle) -> Result<PathBuf, String> {
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|e| format!("could not resolve the app data directory: {e}"))?
            .join("session"))
    }

    pub(super) fn write_token(app: &AppHandle, token: &str) -> Result<(), String> {
        write_file(&session_path(app)?, token)
    }

    pub(super) fn read_token(app: &AppHandle) -> Result<Option<String>, String> {
        read_or_migrate(
            &session_path(app)?,
            super::read_keychain,
            super::delete_keychain,
        )
    }

    /// Signing out truncates the file rather than removing it — the file's existence is what records
    /// that migration already happened.
    pub(super) fn clear_token(app: &AppHandle) -> Result<(), String> {
        write_file(&session_path(app)?, "")
    }
}

/// Windows and Linux keep the OS credential store; see [`store`] on macOS for why it differs there.
#[cfg(not(target_os = "macos"))]
mod store {
    use tauri::AppHandle;

    pub(super) fn write_token(_app: &AppHandle, token: &str) -> Result<(), String> {
        super::entry()?
            .set_password(token)
            .map_err(|e| format!("could not save to the keychain: {e}"))
    }

    pub(super) fn read_token(_app: &AppHandle) -> Result<Option<String>, String> {
        super::read_keychain()
    }

    pub(super) fn clear_token(_app: &AppHandle) -> Result<(), String> {
        super::delete_keychain()
    }
}

/// Removes the keychain entry. A missing entry is not an error.
fn delete_keychain() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not clear the keychain: {e}")),
    }
}

/// Stores the session token, replacing any previous one.
#[tauri::command]
pub fn store_token(app: tauri::AppHandle, token: String) -> Result<(), String> {
    store::write_token(&app, &token)?;
    *lock(&CACHED_TOKEN) = Some(Some(token));
    Ok(())
}

/// Reads the stored session token, or `None` when nobody is signed in.
///
/// Reads the underlying store once per process; see [`CACHED_TOKEN`].
#[tauri::command]
pub fn get_token(app: tauri::AppHandle) -> Result<Option<String>, String> {
    cached_or_read(&CACHED_TOKEN, || store::read_token(&app))
}

/// Removes the stored token. Signing out when already signed out is not an error.
#[tauri::command]
pub fn delete_token(app: tauri::AppHandle) -> Result<(), String> {
    store::clear_token(&app)?;
    *lock(&CACHED_TOKEN) = Some(None);
    Ok(())
}

/// Runs the interactive half of sign-in and returns the one-time code.
///
/// Binds the loopback listener *before* opening the browser, so the redirect cannot arrive before
/// we are ready to catch it. Resolves once the OAuth callback redirects back with `?code=…`; the
/// caller exchanges that for the real token.
///
/// `api_base` is passed in rather than duplicated here — the frontend already resolves it
/// (`web/src/lib/api.ts`), and having one source avoids the two disagreeing.
#[tauri::command]
pub async fn begin_login(api_base: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Claim the flow before binding: any attempt already waiting sees a newer generation on its
        // next poll and releases the port, which is what makes the bind below succeed.
        let generation = LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

        let listener = bind_with_takeover(LOOPBACK_ADDR)?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("could not configure the sign-in listener: {e}"))?;

        // Binds this sign-in attempt to this process. The loopback port is reachable by anything
        // on the machine — and by any page the user is browsing, via a no-cors GET — so without a
        // nonce the listener would accept a code injected by someone else, exchange it, and store
        // *their* session in this user's keychain. RFC 8252 §8.1.
        let nonce = new_nonce();

        let url = format!(
            "{}/api/v1/auth/vatsim/login?desktop=true&desktop_state={}&return_to={}",
            api_base.trim_end_matches('/'),
            urlencode(&nonce),
            urlencode(LOOPBACK_REDIRECT)
        );
        open::that(&url).map_err(|e| format!("could not open your browser: {e}"))?;

        wait_for_code(&listener, &nonce, || {
            LOGIN_GENERATION.load(Ordering::SeqCst) != generation
        })
    })
    .await
    .map_err(|e| format!("sign-in task failed: {e}"))?
}

/// Accepts loopback connections until one carries a `code` **matching this attempt's nonce**, or
/// we give up.
///
/// A browser may open more than one connection (a speculative one, a favicon fetch), so anything
/// without a code is answered and ignored rather than treated as the answer. A request carrying a
/// code but the wrong `state` is likewise ignored: it did not come from the flow we started, so
/// exchanging it would sign this user in as whoever did.
fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    superseded: impl Fn() -> bool,
) -> Result<String, String> {
    wait_for_code_until(listener, expected_state, superseded, LOGIN_TIMEOUT)
}

/// [`wait_for_code`] with the timeout as a parameter, so the timeout path can be tested without
/// waiting five minutes — the same reason `superseded` is a predicate rather than a read of
/// `LOGIN_GENERATION`. Production always goes through `wait_for_code` and `LOGIN_TIMEOUT`.
fn wait_for_code_until(
    listener: &TcpListener,
    expected_state: &str,
    superseded: impl Fn() -> bool,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        // A newer attempt has claimed the flow, so stand down and let it have the port. Returning
        // here drops the listener; holding on would make the new attempt's bind fail (#428). Taken
        // as a predicate rather than read from `LOGIN_GENERATION` here so the wait can be tested
        // without reaching for process-wide state.
        if superseded() {
            // Tell the tab before dropping the listener, rather than leaving it on "Waiting for
            // sign-in" with nothing coming (#536).
            drain_with(listener, CallbackState::Superseded);
            return Err("Sign-in was restarted.".into());
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                // `accept()` on BSD-derived systems (macOS) hands back a socket that INHERITS the
                // listener's O_NONBLOCK. Left that way, a browser — which opens the connection a
                // beat before it sends the request line — reads as `WouldBlock`, which used to be
                // laundered into an empty request and answered as "no code here", silently
                // discarding the real callback and hanging until the timeout. Put the accepted
                // socket back into blocking mode with a short deadline so a slow client is waited
                // for, and a genuinely dead one still can't stall the loop.
                // A failure here is exactly the "callback hangs" bug above, so it is logged.
                if let Err(e) = stream
                    .set_nonblocking(false)
                    .and_then(|()| stream.set_read_timeout(Some(PER_CONNECTION_READ_TIMEOUT)))
                {
                    log::warn!("sign-in callback socket could not be made blocking: {e}");
                }

                let request = match read_request_head(&mut stream) {
                    Ok(request) => request,
                    // A read error is NOT the same as "carried no code" — say so and move on
                    // rather than treating this connection as an answered non-callback.
                    // Never the request text: it carries the one-time code and state.
                    Err(e) => {
                        log::warn!("sign-in callback request could not be read: {e}");
                        respond(&mut stream, CallbackState::Waiting);
                        continue;
                    }
                };

                match request_code(&request) {
                    Some(code)
                        if request_param(&request, "state").as_deref() == Some(expected_state) =>
                    {
                        respond(&mut stream, CallbackState::SignedIn);
                        return Ok(code);
                    }
                    // Either no code at all, or a code from someone else's flow.
                    _ => respond(&mut stream, CallbackState::Waiting),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(format!("sign-in listener failed: {e}")),
        }
    }

    drain_with(listener, CallbackState::TimedOut);
    Err("Sign-in timed out. Please try again.".into())
}

/// Reads until the request line is complete (or the head ends), rather than assuming a single
/// `read` returns it. A short read is normal on a socket; treating one as the whole request is how
/// a callback gets dropped.
fn read_request_head(stream: &mut impl Read) -> std::io::Result<String> {
    let mut collected: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..read]);

        // The request line is all we parse; stop as soon as we have it.
        if collected.contains(&b'\n') || collected.len() >= 8192 {
            break;
        }
    }

    Ok(String::from_utf8_lossy(&collected).into_owned())
}

/// Pulls `code` out of the request line (`GET /callback?code=… HTTP/1.1`).
fn request_code(request: &str) -> Option<String> {
    request_param(request, "code")
}

/// Pulls one named query parameter out of the request line.
fn request_param(request: &str, name: &str) -> Option<String> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let query = target.split_once('?')?.1;

    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name && !value.is_empty()).then(|| urldecode(value))
    })
}

/// A random nonce for one sign-in attempt. Two v4 UUIDs, matching how the backend mints the
/// one-time code itself — both are CSPRNG-backed.
fn new_nonce() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// What the sign-in tab is being told (VATUSA/OIS#536).
///
/// An enum rather than a message string so every state is spelled out in one place and the compiler
/// requires a page for each. Two of these previously had no page at all: the tab was simply never
/// answered, so a user who waited too long sat on "Waiting for sign-in..." indefinitely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallbackState {
    /// The code arrived and matched the expected `state`.
    SignedIn,
    /// A request reached the listener that was not the callback, or could not be read.
    Waiting,
    /// `LOGIN_TIMEOUT` elapsed before a usable callback arrived.
    TimedOut,
    /// A newer sign-in attempt claimed the flow.
    Superseded,
}

impl CallbackState {
    /// Heading and body copy. Written as statements of what happened, and what to do about it —
    /// a dead end with no instruction is what made the unanswered states feel broken.
    fn copy(self) -> (&'static str, &'static str) {
        match self {
            Self::SignedIn => ("Signed in", "You can close this tab and return to OIS."),
            Self::Waiting => (
                "Waiting for sign-in",
                "Finish signing in with VATSIM in the other tab. This page won't change — if OIS \
                 doesn't sign you in, start again from the app.",
            ),
            Self::TimedOut => (
                "Sign-in timed out",
                "This sign-in attempt expired. Close this tab and start again from OIS.",
            ),
            Self::Superseded => (
                "Sign-in restarted",
                "Another sign-in attempt took over. Close this tab and continue in the newer one.",
            ),
        }
    }

    /// The accent for the status dot. Only `SignedIn` departs from the neutral ink, so the page has
    /// one accent at a time — `DESIGN.md`'s rule, not a palette.
    fn dot(self) -> &'static str {
        match self {
            Self::SignedIn => "var(--success)",
            Self::Waiting => "var(--brand)",
            Self::TimedOut | Self::Superseded => "var(--warning)",
        }
    }
}

/// Escape text for interpolation into HTML.
///
/// Every call site passes a fixed literal today, so nothing here is currently injectable — but this
/// function exists so that stays true by construction rather than by review. An OAuth `error`
/// parameter echoed into the page would otherwise be script injection into a page served on
/// loopback (#536).
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The sign-in tab's page, as one self-contained document.
///
/// ## Why the tokens are inlined, and the `DESIGN.md` exception
///
/// `DESIGN.md` § "The 9 non-negotiables" #9 says *"Tokens only — never inline a value… A hardcoded
/// hex or px in a component is a bug"*, and § "The shell" says the signed-out homepage is the only
/// page outside the app shell. This page breaks both, deliberately and with the exception recorded
/// in `DESIGN.md` § "Standalone pages outside the app":
///
/// - It is served by the Rust loopback listener on `127.0.0.1:8765`, not by the web app, so it can
///   reach neither `packages/ui`'s stylesheet nor the shell.
/// - It must render with **no network at all**. The fonts are self-hosted through `@fontsource` and
///   imported by the web entry point, so they are unreachable from here; the stacks below name
///   Inter and JetBrains Mono first for the machines that have them and fall back to system UI.
///
/// The colours below are a **copy**. `packages/ui/src/styles/globals.css` (`.dark`) is the source of
/// truth, and `the_inlined_tokens_match_the_stylesheet` reads it: if a token moves there, that test
/// fails until this copy follows. `--r-lg` comes from `DESIGN.md`'s token table instead — the
/// stylesheet does not define it — and the font stacks approximate `--font-sans`/`--font-mono` with
/// system fallbacks, since this page has no network to load the real faces.
///
/// ## Dark-only
///
/// No `prefers-color-scheme` branch. The app window this tab hands back to is unconditionally dark
/// (`tauri.conf.json`'s `backgroundColor: "#08080a"`, which is `--ground`), so honouring a light OS
/// preference here would flash a light page on the way into a dark app and read as a glitch.
fn callback_page(state: CallbackState) -> String {
    let (heading, detail) = state.copy();
    let (heading, detail) = (escape_html(heading), escape_html(detail));
    let dot = state.dot();
    format!(
        r##"<!doctype html>
<html lang="en" style="color-scheme: dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>OIS — {heading}</title>
<style>
  /* Colours copied from packages/ui/src/styles/globals.css (.dark) and held to it by a test; --r-lg from DESIGN.md; font stacks approximate. See callback_page's docs. */
  :root {{
    --ground: #08080a;
    --card: #16161b;
    --line: #26262d;
    --ink: #f3f3f5;
    --ink-2: #a1a1aa;
    --brand: #6ea8fe;
    --success: #43d089;
    --warning: #efc14d;
    --r-lg: 16px;
    --sans: Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
    --mono: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    min-height: 100vh;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--ground);
    color: var(--ink);
    font: 400 15px/1.5 var(--sans);
    -webkit-font-smoothing: antialiased;
  }}
  /* Elevation is a surface step plus a hairline — never a shadow (DESIGN.md). */
  main {{
    width: 100%;
    max-width: 420px;
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    padding: 28px;
  }}
  .mark {{
    font: 700 12px/1 var(--mono);
    letter-spacing: 0.14em;
    color: var(--ink-2);
    text-transform: uppercase;
  }}
  h1 {{
    margin: 18px 0 0;
    font: 600 22px/1.25 var(--sans);
    display: flex;
    align-items: center;
    gap: 10px;
  }}
  /* The one accent on the page, sized to read as a status rather than decoration. */
  .dot {{
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: {dot};
    flex: none;
  }}
  p {{ margin: 10px 0 0; color: var(--ink-2); }}
</style>
</head>
<body>
<main>
  <div class="mark">VATUSA OIS</div>
  <h1><span class="dot" aria-hidden="true"></span>{heading}</h1>
  <p>{detail}</p>
</main>
</body>
</html>"##
    )
}

/// Answer one connection with the page for `state`.
fn respond(stream: &mut impl Write, state: CallbackState) {
    let body = callback_page(state);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Answer whatever is already queued on `listener` with `state`, then give up the port.
///
/// The timeout and superseded paths used to return without answering anything, so a callback that
/// landed in the instant we gave up got a connection reset and the browser showed its own error
/// page. This drains what is pending — bounded, because the listener is non-blocking and
/// `WouldBlock` ends the sweep — so a late callback is told what happened instead.
///
/// It cannot update a tab that is *already* showing "Waiting for sign-in": HTTP has no way to push
/// to a response that was already written, and polling from that page would only reach a listener
/// that is in the act of closing. What this fixes is the connection arriving around the deadline.
fn drain_with(listener: &TcpListener, state: CallbackState) {
    // Set here, not assumed. `bind_with_takeover` returns a *blocking* listener — `begin_login`
    // makes it non-blocking separately — so a drain that trusted its caller would block forever in
    // `accept()` the moment the queue emptied. Harmless to repeat on an already non-blocking socket,
    // and the listener is about to be dropped by every caller anyway.
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    // A cap as well as the WouldBlock exit: a client reconnecting in a tight loop must not be able
    // to keep the port held open past the deadline it just expired on.
    for _ in 0..16 {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(PER_CONNECTION_READ_TIMEOUT));
                // The request is read and discarded: whatever it asked for, the flow is over and
                // this is the only answer there is. Reading it first keeps the client from seeing a
                // reset on an unread socket.
                let _ = read_request_head(&mut stream);
                respond(&mut stream, state);
            }
            _ => return,
        }
    }
}

/// Percent-encodes the few characters that matter for the one URL we build.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Reverses percent-encoding in a query value. The code is hex, but a proxy or browser is free to
/// escape it anyway, so decode rather than assume.
///
/// The two digits after `%` are read from the *bytes*, never by slicing the `&str`: this is network
/// input from any local client, and a `%` followed by a multi-byte character would put a `str` slice
/// boundary inside that character and panic (VATUSA/OIS#346 review).
fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' => match hex_byte(bytes.get(i + 1..i + 3)) {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// The byte two hex digits spell, if `digits` is exactly two ASCII hex digits.
fn hex_byte(digits: Option<&[u8]>) -> Option<u8> {
    u8::from_str_radix(std::str::from_utf8(digits?).ok()?, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ephemeral port to exercise the bind with, so the suite never touches the real sign-in
    /// port — something else on the machine holding 8765 would otherwise make these fail.
    fn spare_port() -> String {
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind a spare port");
        let addr = probe.local_addr().expect("read the spare port").to_string();
        drop(probe);
        addr
    }

    #[test]
    fn binds_the_loopback_listener_when_the_port_is_free() {
        let addr = spare_port();
        assert!(bind_with_takeover(&addr).is_ok());
    }

    /// #428: a second sign-in used to fail outright here, and because the first attempt holds the
    /// port for the rest of `LOGIN_TIMEOUT`, the user was wedged for five minutes.
    #[test]
    fn waits_for_a_superseded_attempt_to_release_the_port() {
        let addr = spare_port();
        let held = TcpListener::bind(&addr).expect("hold the port");

        let releasing = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(held);
        });

        let started = Instant::now();
        let listener = bind_with_takeover(&addr);
        releasing.join().expect("releasing thread");

        assert!(
            listener.is_ok(),
            "should have taken the port over: {listener:?}"
        );
        // It waited rather than racing through, and did not sit out the whole deadline.
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < TAKEOVER_TIMEOUT);
    }

    /// A port held by something that is *not* a superseded sign-in is not worth waiting on, and the
    /// message has to name the address so the user can find the culprit.
    ///
    /// Calls the real retry with an already-spent deadline, so it gives up at once rather than
    /// sitting out `TAKEOVER_TIMEOUT`. The previous version of this test built the expected string
    /// itself and asserted it started with a prefix of itself — it passed against an entirely
    /// different message (#428 review).
    #[test]
    fn reports_a_port_that_never_frees_up() {
        let addr = spare_port();
        let _held = TcpListener::bind(&addr).expect("hold the port");

        let spent = Instant::now() - Duration::from_secs(1);
        let err = bind_until(&addr, spent).expect_err("the port is held, so this cannot bind");

        assert!(
            err.starts_with(&format!("could not listen on {addr}")),
            "the error must name the address the user has to free up, got: {err}"
        );
    }

    /// The give-up path must not be reached while the deadline is still running — that is the whole
    /// difference between waiting a superseded attempt out and failing the way #428 did.
    #[test]
    fn waits_rather_than_giving_up_while_the_deadline_is_live() {
        let addr = spare_port();
        let _held = TcpListener::bind(&addr).expect("hold the port");

        let started = Instant::now();
        let err = bind_until(&addr, started + Duration::from_millis(200));

        assert!(err.is_err());
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "it gave up early rather than retrying until the deadline"
        );
    }

    /// The generation is what makes the takeover possible: an attempt that no longer owns the flow
    /// must stop waiting, because holding the listener is what blocked the new attempt's bind.
    #[test]
    fn a_superseded_attempt_stands_down_instead_of_holding_the_port() {
        let addr = spare_port();
        let listener = bind_with_takeover(&addr).expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");

        let started = Instant::now();
        let result = wait_for_code(&listener, "nonce", || true);

        assert_eq!(result.unwrap_err(), "Sign-in was restarted.");
        // Immediately, rather than after LOGIN_TIMEOUT.
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// Network input: a `%` before a multi-byte character used to slice mid-character and panic.
    #[test]
    fn a_percent_before_a_multibyte_character_is_kept_not_a_panic() {
        assert_eq!(urldecode("%€"), "%€");
        assert_eq!(urldecode("a%é1"), "a%é1");
        assert_eq!(urldecode("%4"), "%4");
        assert_eq!(urldecode("%41%42"), "AB");
    }

    #[test]
    fn pulls_the_code_out_of_the_callback_request() {
        let request = "GET /callback?code=abc123 HTTP/1.1\r\nHost: 127.0.0.1:8765\r\n\r\n";
        assert_eq!(request_code(request).as_deref(), Some("abc123"));
    }

    #[test]
    fn ignores_a_request_that_carries_no_code() {
        // A browser will happily ask for this before the redirect lands; treating it as the answer
        // would abort sign-in.
        let request = "GET /favicon.ico HTTP/1.1\r\n\r\n";
        assert_eq!(request_code(request), None);
        assert_eq!(
            request_code("GET /callback?error=denied HTTP/1.1\r\n\r\n"),
            None
        );
        assert_eq!(request_code("GET /callback?code= HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn finds_the_code_among_other_params() {
        let request = "GET /callback?state=xyz&code=deadbeef HTTP/1.1\r\n\r\n";
        assert_eq!(request_code(request).as_deref(), Some("deadbeef"));
    }

    #[test]
    fn decodes_a_percent_encoded_code() {
        let request = "GET /callback?code=a%2Bb%20c HTTP/1.1\r\n\r\n";
        assert_eq!(request_code(request).as_deref(), Some("a+b c"));
    }

    #[test]
    fn encodes_the_redirect_so_it_survives_as_a_query_value() {
        assert_eq!(
            urlencode(LOOPBACK_REDIRECT),
            "http%3A%2F%2F127.0.0.1%3A8765%2Fcallback"
        );
    }

    /// The regression that made sign-in fail on macOS: `accept()` there hands back a socket that
    /// inherits the listener's O_NONBLOCK, so a browser — which opens the connection a beat before
    /// it sends the request line — used to read as `WouldBlock`, get laundered into an empty
    /// request, and have its code thrown away. This client deliberately delays.
    #[test]
    fn waits_for_a_client_that_sends_its_request_late() {
        use std::net::TcpStream;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        std::thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            std::thread::sleep(Duration::from_millis(400));
            let _ = s.write_all(b"GET /callback?code=REAL&state=NONCE HTTP/1.1\r\nHost: x\r\n\r\n");
            std::thread::sleep(Duration::from_millis(200));
        });

        assert_eq!(wait_for_code(&listener, "NONCE", || false).unwrap(), "REAL");
    }

    /// A code that did not come from the flow this process started must be ignored. Without this,
    /// anything able to reach the loopback port — a local process, or a page the user is browsing
    /// issuing a no-cors GET — could have its own code exchanged and its own session written into
    /// this user's keychain.
    #[test]
    fn ignores_a_code_from_someone_elses_flow() {
        use std::net::TcpStream;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        std::thread::spawn(move || {
            // Injected: no state at all, then a wrong one. Neither may be accepted.
            for req in [
                &b"GET /callback?code=ATTACKER HTTP/1.1\r\nHost: x\r\n\r\n"[..],
                &b"GET /callback?code=ATTACKER&state=WRONG HTTP/1.1\r\nHost: x\r\n\r\n"[..],
            ] {
                if let Ok(mut s) = TcpStream::connect(addr) {
                    let _ = s.write_all(req);
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            // ...then the real one.
            if let Ok(mut s) = TcpStream::connect(addr) {
                let _ =
                    s.write_all(b"GET /callback?code=MINE&state=NONCE HTTP/1.1\r\nHost: x\r\n\r\n");
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        assert_eq!(
            wait_for_code(&listener, "NONCE", || false).unwrap(),
            "MINE",
            "only the code carrying this attempt's nonce may be accepted"
        );
    }

    #[test]
    fn a_nonce_is_unique_per_attempt() {
        assert_ne!(new_nonce(), new_nonce());
        assert_eq!(new_nonce().len(), 64);
    }

    // --- the sign-in tab's page (VATUSA/OIS#536) ---------------------------------------------

    /// Every state, so a new variant cannot be added without deciding what its page says.
    const ALL_STATES: [CallbackState; 4] = [
        CallbackState::SignedIn,
        CallbackState::Waiting,
        CallbackState::TimedOut,
        CallbackState::Superseded,
    ];

    /// AC1: a real document, not a bare `<p>`. The old reply had no `<html>`, `<head>`, `<body>`,
    /// viewport or `lang`, which is why the browser fell back to white and Times New Roman.
    #[test]
    fn every_state_renders_a_complete_document() {
        for state in ALL_STATES {
            let html = callback_page(state);
            for required in [
                "<!doctype html>",
                "<html lang=\"en\"",
                "<head>",
                "<body>",
                "name=\"viewport\"",
                "charset=\"utf-8\"",
            ] {
                assert!(html.contains(required), "{state:?} is missing {required}");
            }
        }
    }

    /// AC3: self-contained. A fetch of any kind would leave the page unstyled offline — which is
    /// the state it is in today, and the whole complaint.
    #[test]
    fn the_page_fetches_nothing() {
        for state in ALL_STATES {
            let html = callback_page(state);
            for forbidden in [
                "<link", "<script", "@import", "http://", "https://", "//fonts",
            ] {
                assert!(
                    !html.contains(forbidden),
                    "{state:?} would reach the network via {forbidden}"
                );
            }
        }
    }

    /// The page's token copy, held to the stylesheet it was copied from.
    ///
    /// `DESIGN.md` allows this page to inline tokens only because a test stands between the copy and
    /// drift. The test that used to sit here asserted the page contained hex values typed into the
    /// *test*, which pinned one copy to another: changing `--card` in `globals.css` left all 32 tests
    /// green (#536 review). This reads the stylesheet itself, so moving a token there fails here until
    /// the page follows.
    #[test]
    fn the_inlined_tokens_match_the_stylesheet() {
        const STYLESHEET: &str = include_str!("../../../packages/ui/src/styles/globals.css");
        // Inlined with no stylesheet counterpart, each deliberately. Anything else the page inlines
        // must come from `.dark`, so a new copied token cannot slip in unchecked.
        const NOT_FROM_THE_STYLESHEET: &[&str] = &[
            "--r-lg", // `DESIGN.md`'s token table; `globals.css` does not define it
            "--sans", // approximates `--font-sans` with system fallbacks: this page has no network
            "--mono", // approximates `--font-mono`, for the same reason
        ];

        let stylesheet = declarations(block_after(STYLESHEET, "\n.dark {"));
        let html = callback_page(CallbackState::Waiting);
        let page = declarations(block_after(&html, ":root {"));
        assert!(
            page.len() >= 6,
            "the page's token block was not parsed: {page:?}"
        );

        for (name, value) in &page {
            if NOT_FROM_THE_STYLESHEET.contains(&name.as_str()) {
                continue;
            }
            let source = stylesheet.get(name).unwrap_or_else(|| {
                panic!(
                    "{name} is inlined but globals.css's .dark block does not define it — copy it \
                     from there, or list it with a reason in NOT_FROM_THE_STYLESHEET"
                )
            });
            assert_eq!(value, source, "{name} has drifted from globals.css");
        }
    }

    /// The text of the first `{ … }` block after `opener`.
    fn block_after<'a>(text: &'a str, opener: &str) -> &'a str {
        let start = text
            .find(opener)
            .unwrap_or_else(|| panic!("no `{opener}` block"))
            + opener.len();
        let end = text[start..].find('}').expect("an unterminated block") + start;
        &text[start..end]
    }

    /// The custom properties declared in a CSS block, with `/* … */` comments removed first — a
    /// comment directly before a declaration would otherwise swallow its name and drop it silently.
    fn declarations(block: &str) -> std::collections::BTreeMap<String, String> {
        let mut code = String::with_capacity(block.len());
        let mut rest = block;
        while let Some(open) = rest.find("/*") {
            code.push_str(&rest[..open]);
            rest = rest[open..]
                .find("*/")
                .map_or("", |close| &rest[open + close + 2..]);
        }
        code.push_str(rest);
        code.split(';')
            .filter_map(|declaration| {
                let (name, value) = declaration.trim().split_once(':')?;
                let name = name.trim();
                name.starts_with("--")
                    .then(|| (name.to_string(), value.trim().to_string()))
            })
            .collect()
    }

    #[test]
    fn declarations_skip_comments_rather_than_the_token_after_them() {
        let parsed = declarations("/* surfaces */ --ground: #08080a; --card:  #16161b ;");
        assert_eq!(parsed.get("--ground").map(String::as_str), Some("#08080a"));
        assert_eq!(parsed.get("--card").map(String::as_str), Some("#16161b"));
    }

    /// `DESIGN.md`: elevation is a surface step plus a hairline, never a shadow, and never a
    /// gradient. Easy to reintroduce by habit when hand-writing CSS.
    #[test]
    fn the_page_has_no_shadows_or_gradients() {
        for state in ALL_STATES {
            let html = callback_page(state);
            assert!(!html.contains("box-shadow"), "{state:?} has a shadow");
            assert!(!html.contains("gradient"), "{state:?} has a gradient");
        }
    }

    /// `DESIGN.md`'s type ladder is 400/600/700 with 500 banned.
    #[test]
    fn the_page_stays_on_the_weight_ladder() {
        for state in ALL_STATES {
            let html = callback_page(state);
            for banned in ["font-weight: 500", "font: 500", "font-weight:500"] {
                assert!(!html.contains(banned), "{state:?} uses a banned weight");
            }
        }
    }

    /// AC4: deliberately dark-only, declared rather than left to the browser. `color-scheme`
    /// is what stops a light-mode browser painting white scrollbars and form chrome around it.
    #[test]
    fn the_page_declares_itself_dark() {
        let html = callback_page(CallbackState::SignedIn);
        assert!(html.contains("color-scheme: dark"));
        assert!(
            !html.contains("prefers-color-scheme"),
            "dark-only is the decision; a light branch would flash before a dark app window"
        );
    }

    /// Each state says something different, and says what to do. The two formerly-unanswered
    /// states must not read as "waiting", which is the dead end #536 describes.
    #[test]
    fn each_state_says_something_distinct_and_actionable() {
        let mut headings = std::collections::HashSet::new();
        for state in ALL_STATES {
            let (heading, detail) = state.copy();
            assert!(headings.insert(heading), "{state:?} reuses a heading");
            assert!(!detail.is_empty(), "{state:?} has no instruction");
        }
        assert!(CallbackState::TimedOut.copy().0.contains("timed out"));
        assert!(
            CallbackState::Superseded
                .copy()
                .1
                .contains("Close this tab")
        );
    }

    /// AC5. Not exploitable today — all three call sites pass literals — but the escaping is what
    /// keeps that true when a dynamic message (an OAuth `error`, say) is eventually interpolated.
    #[test]
    fn html_is_escaped() {
        assert_eq!(
            escape_html("<script>alert('x')</script>"),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"
        );
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(
            escape_html("plain text"),
            "plain text",
            "nothing else is touched"
        );
    }

    /// The ampersand must be escaped first, or `<` becomes `&amp;lt;`.
    #[test]
    fn escaping_does_not_double_encode() {
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
    }

    /// The HTTP framing has to match the body, or the browser hangs waiting for bytes that never
    /// come. Worth pinning because the body is no longer a one-liner whose length is obvious.
    #[test]
    fn the_response_declares_its_real_length() {
        let mut out: Vec<u8> = Vec::new();
        respond(&mut out, CallbackState::SignedIn);
        let text = String::from_utf8(out).expect("utf-8");

        let (head, body) = text.split_once("\r\n\r\n").expect("a header/body split");
        let declared: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .expect("a Content-Length")
            .trim()
            .parse()
            .expect("a number");
        assert_eq!(declared, body.len(), "Content-Length must match the body");
        assert!(head.contains("text/html; charset=utf-8"));
        assert!(head.starts_with("HTTP/1.1 200 OK"));
    }

    /// A browser tab sitting on the callback: connects, sends a request, and reads whatever comes
    /// back. Returns the whole response once the server closes the connection.
    fn waiting_tab(addr: String) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(&addr).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            stream
                .write_all(b"GET /callback HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("write");
            let mut got = String::new();
            let _ = stream.read_to_string(&mut got);
            got
        })
    }

    /// AC2, through the real exit rather than through `drain_with` directly. The timeout path used to
    /// return without answering anything; a tab connected at that moment got a reset and the
    /// browser's own error page. A zero timeout skips the wait loop entirely and goes straight to
    /// that exit — deterministically, with no race against the loop's own `accept`.
    #[test]
    fn a_timed_out_flow_answers_the_waiting_tab() {
        let addr = spare_port();
        let listener = bind_with_takeover(&addr).expect("bind");
        let tab = waiting_tab(addr);
        std::thread::sleep(Duration::from_millis(150)); // let it reach the backlog

        let result = wait_for_code_until(&listener, "nonce", || false, Duration::ZERO);

        assert_eq!(result.unwrap_err(), "Sign-in timed out. Please try again.");
        let got = tab.join().expect("tab thread");
        assert!(
            got.starts_with("HTTP/1.1 200 OK"),
            "got: {}",
            &got[..got.len().min(60)]
        );
        assert!(
            got.contains("Sign-in timed out"),
            "the tab is told it timed out"
        );
    }

    /// AC2, the other unanswered exit. `superseded` is checked before `accept`, so this also proves
    /// the drain runs *before* the listener is dropped — dropping first would reset the tab.
    #[test]
    fn a_superseded_flow_answers_the_waiting_tab() {
        let addr = spare_port();
        let listener = bind_with_takeover(&addr).expect("bind");
        let tab = waiting_tab(addr);
        std::thread::sleep(Duration::from_millis(150));

        let result = wait_for_code(&listener, "nonce", || true);

        assert_eq!(result.unwrap_err(), "Sign-in was restarted.");
        let got = tab.join().expect("tab thread");
        assert!(
            got.contains("Sign-in restarted"),
            "the tab is told it was superseded"
        );
        assert!(
            !got.contains("timed out"),
            "and not given the timeout page, which would send them to the wrong fix"
        );
    }

    /// With nothing queued it must return immediately rather than blocking on `accept`. Deliberately
    /// given a **blocking** listener, straight from `bind_with_takeover`: that is what this test
    /// first caught — the drain used to trust its caller to have set non-blocking, and would have
    /// hung the suite forever.
    #[test]
    fn draining_an_idle_listener_returns_at_once() {
        let addr = spare_port();
        let listener = bind_with_takeover(&addr).expect("bind");

        let start = Instant::now();
        drain_with(&listener, CallbackState::Superseded);

        assert!(
            start.elapsed() < Duration::from_secs(1),
            "an idle drain must not wait for a connection"
        );
    }

    /// AC5: one keychain read per process, however many webviews ask. Each pop-out and each restored
    /// route window used to make its own call, and on macOS every call is ACL-gated and can prompt.
    #[test]
    fn the_keychain_is_read_once_however_many_callers() {
        let cache = Mutex::new(None);
        let reads = std::cell::Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            Ok(Some("ois_dsk_abc".to_string()))
        };

        assert_eq!(
            cached_or_read(&cache, read).unwrap(),
            Some("ois_dsk_abc".to_string())
        );
        assert_eq!(
            cached_or_read(&cache, read).unwrap(),
            Some("ois_dsk_abc".to_string())
        );
        assert_eq!(reads.get(), 1, "second caller must not reach the keychain");
    }

    /// Signed out is an answer worth caching: re-reading it is another ACL-gated hit for a result
    /// that cannot have changed without `store_token`.
    #[test]
    fn an_empty_keychain_is_cached_rather_than_re_read() {
        let cache = Mutex::new(None);
        let reads = std::cell::Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            Ok(None)
        };

        assert_eq!(cached_or_read(&cache, read).unwrap(), None);
        assert_eq!(cached_or_read(&cache, read).unwrap(), None);
        assert_eq!(reads.get(), 1);
    }

    /// A failure must NOT be cached here. The retry loop is stopped in the frontend; caching the
    /// error in the shell as well would mean a transient keychain fault could never recover without
    /// restarting the app.
    #[test]
    fn a_failed_read_is_not_cached_so_it_can_recover() {
        let cache = Mutex::new(None);
        let attempts = std::cell::Cell::new(0);

        let first = cached_or_read(&cache, || {
            attempts.set(attempts.get() + 1);
            Err("keychain unavailable".to_string())
        });
        assert!(first.is_err());

        let second = cached_or_read(&cache, || {
            attempts.set(attempts.get() + 1);
            Ok(Some("ois_dsk_recovered".to_string()))
        });
        assert_eq!(second.unwrap(), Some("ois_dsk_recovered".to_string()));
        assert_eq!(attempts.get(), 2, "a failure must leave the cache unset");
    }

    /// A poisoned lock must not take the keychain down with it — the cached value is a plain
    /// `Option<String>` with no invariant for poisoning to protect.
    #[test]
    fn a_poisoned_cache_still_serves() {
        let cache = Mutex::new(Some(Some("ois_dsk_abc".to_string())));
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache.lock().unwrap();
            panic!("poison it");
        });
        assert!(cache.is_poisoned());

        assert_eq!(
            cached_or_read(&cache, || panic!("must not read")).unwrap(),
            Some("ois_dsk_abc".to_string())
        );
    }

    /// The macOS file store and the one-time migration off the keychain (#535 AC1). These exercise
    /// the pure path/closure forms, so no test touches a real keychain or the app data directory.
    #[cfg(target_os = "macos")]
    mod macos_store {
        use std::{cell::Cell, fs, os::unix::fs::PermissionsExt, path::PathBuf};

        use super::super::store::{Stored, read_file, read_or_migrate, write_file};

        /// A unique scratch path per test, so cases cannot collide.
        fn scratch(name: &str) -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("ois-session-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            dir.join("session")
        }

        fn mode_of(path: &PathBuf) -> u32 {
            fs::metadata(path).expect("stat").permissions().mode() & 0o777
        }

        #[test]
        fn a_token_round_trips_through_the_file() {
            let path = scratch("round-trip");
            write_file(&path, "ois_dsk_abc").unwrap();
            assert_eq!(
                read_file(&path).unwrap(),
                Stored::Token("ois_dsk_abc".into())
            );
        }

        /// Permissions are the entire protection for this file, so the bits are asserted rather than
        /// assuming a successful write implies a safe one.
        #[test]
        fn the_file_is_owner_only() {
            let path = scratch("mode");
            write_file(&path, "ois_dsk_abc").unwrap();
            assert_eq!(
                mode_of(&path),
                0o600,
                "the token must not be world- or group-readable"
            );
        }

        /// And a file left loose by anything else is tightened on the next write, not trusted.
        #[test]
        fn an_existing_loose_file_is_tightened() {
            let path = scratch("tighten");
            write_file(&path, "first").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

            write_file(&path, "second").unwrap();
            assert_eq!(mode_of(&path), 0o600);
        }

        #[test]
        fn an_empty_file_means_signed_out_not_missing() {
            let path = scratch("empty");
            write_file(&path, "").unwrap();
            assert_eq!(read_file(&path).unwrap(), Stored::SignedOut);
        }

        #[test]
        fn no_file_at_all_means_not_yet_migrated() {
            assert_eq!(read_file(&scratch("absent")).unwrap(), Stored::Missing);
        }

        /// The load-bearing one: an upgrading user keeps their session, and the keychain entry is
        /// removed so no stale bearer token is left behind.
        #[test]
        fn the_first_read_migrates_the_token_out_of_the_keychain() {
            let path = scratch("migrate");
            let deleted = Cell::new(false);

            let token = read_or_migrate(
                &path,
                || Ok(Some("ois_dsk_from_keychain".to_string())),
                || {
                    deleted.set(true);
                    Ok(())
                },
            )
            .unwrap();

            assert_eq!(token.as_deref(), Some("ois_dsk_from_keychain"));
            assert_eq!(
                read_file(&path).unwrap(),
                Stored::Token("ois_dsk_from_keychain".into())
            );
            assert!(deleted.get(), "the keychain entry must not be left behind");
            assert_eq!(mode_of(&path), 0o600);
        }

        /// The regression this change exists to prevent. If migration were re-attempted, every
        /// launch would hit the ACL-gated keychain and prompt again — which is the original bug.
        #[test]
        fn migration_happens_once_even_when_it_finds_nothing() {
            let path = scratch("once");
            let reads = Cell::new(0);

            for _ in 0..3 {
                let token = read_or_migrate(
                    &path,
                    || {
                        reads.set(reads.get() + 1);
                        Ok(None)
                    },
                    || Ok(()),
                )
                .unwrap();
                assert_eq!(token, None);
            }

            assert_eq!(
                reads.get(),
                1,
                "the keychain must be consulted exactly once, ever"
            );
        }

        /// A dismissed prompt reads as signed out *and* still closes migration, so dismissing it
        /// costs one prompt in total rather than one per launch.
        #[test]
        fn a_dismissed_keychain_prompt_reads_as_signed_out_and_still_settles() {
            let path = scratch("dismissed");
            let reads = Cell::new(0);
            let deleted = Cell::new(false);

            let first = read_or_migrate(
                &path,
                || {
                    reads.set(reads.get() + 1);
                    Err("user dismissed the keychain prompt".to_string())
                },
                || {
                    deleted.set(true);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(first, None);
            assert_eq!(read_file(&path).unwrap(), Stored::SignedOut);
            assert!(
                !deleted.get(),
                "deleting is ACL-gated too, so it must not be attempted after a failed read"
            );

            let second =
                read_or_migrate(&path, || panic!("must not read again"), || Ok(())).unwrap();
            assert_eq!(second, None);
            assert_eq!(reads.get(), 1);
        }

        /// Signing out must leave the marker, or the next launch re-migrates and prompts.
        #[test]
        fn signing_out_keeps_the_marker_file() {
            let path = scratch("sign-out");
            write_file(&path, "ois_dsk_abc").unwrap();

            write_file(&path, "").unwrap(); // what clear_token does

            assert!(
                path.exists(),
                "the file is the record that migration already happened"
            );
            assert_eq!(read_file(&path).unwrap(), Stored::SignedOut);
            let token =
                read_or_migrate(&path, || panic!("must not read the keychain"), || Ok(())).unwrap();
            assert_eq!(token, None);
        }

        #[test]
        fn an_unreadable_file_reads_as_signed_out_rather_than_panicking() {
            // A directory where the file should be: readable as neither token nor absent.
            let path = scratch("corrupt");
            fs::create_dir_all(&path).unwrap();
            assert_eq!(read_file(&path).unwrap(), Stored::SignedOut);
        }

        /// Surrounding whitespace is not part of the token — a trailing newline from an editor or an
        /// earlier writer must not change what is sent as the bearer.
        #[test]
        fn surrounding_whitespace_is_not_part_of_the_token() {
            let path = scratch("trim");
            write_file(&path, "  ois_dsk_abc\n").unwrap();
            assert_eq!(
                read_file(&path).unwrap(),
                Stored::Token("ois_dsk_abc".into())
            );
        }
    }
}

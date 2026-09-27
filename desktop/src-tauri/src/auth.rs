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

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| format!("keychain unavailable: {e}"))
}

/// Stores the session token in the OS keychain, replacing any previous one.
#[tauri::command]
pub fn store_token(token: String) -> Result<(), String> {
    entry()?
        .set_password(&token)
        .map_err(|e| format!("could not save to the keychain: {e}"))
}

/// Reads the stored session token, or `None` when nobody is signed in.
///
/// A missing entry is the ordinary signed-out case, not an error.
#[tauri::command]
pub fn get_token() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read the keychain: {e}")),
    }
}

/// Removes the stored token. Signing out when already signed out is not an error.
#[tauri::command]
pub fn delete_token() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not clear the keychain: {e}")),
    }
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
        let listener = TcpListener::bind(LOOPBACK_ADDR)
            .map_err(|e| format!("could not listen on {LOOPBACK_ADDR}: {e}"))?;
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

        wait_for_code(&listener, &nonce)
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
fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // `accept()` on BSD-derived systems (macOS) hands back a socket that INHERITS the
                // listener's O_NONBLOCK. Left that way, a browser — which opens the connection a
                // beat before it sends the request line — reads as `WouldBlock`, which used to be
                // laundered into an empty request and answered as "no code here", silently
                // discarding the real callback and hanging until the timeout. Put the accepted
                // socket back into blocking mode with a short deadline so a slow client is waited
                // for, and a genuinely dead one still can't stall the loop.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(PER_CONNECTION_READ_TIMEOUT));

                let request = match read_request_head(&mut stream) {
                    Ok(request) => request,
                    // A read error is NOT the same as "carried no code" — say so and move on
                    // rather than treating this connection as an answered non-callback.
                    Err(_) => {
                        respond(&mut stream, "Waiting for sign-in...");
                        continue;
                    }
                };

                match request_code(&request) {
                    Some(code)
                        if request_param(&request, "state").as_deref() == Some(expected_state) =>
                    {
                        respond(
                            &mut stream,
                            "Signed in. You can close this tab and return to OIS.",
                        );
                        return Ok(code);
                    }
                    // Either no code at all, or a code from someone else's flow.
                    _ => respond(&mut stream, "Waiting for sign-in..."),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(format!("sign-in listener failed: {e}")),
        }
    }

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

/// A minimal HTML reply, so the user sees something rather than a browser error page.
fn respond(stream: &mut impl Write, message: &str) {
    let body = format!("<!doctype html><meta charset=\"utf-8\"><title>OIS</title><p>{message}</p>");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
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

        assert_eq!(wait_for_code(&listener, "NONCE").unwrap(), "REAL");
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
            wait_for_code(&listener, "NONCE").unwrap(),
            "MINE",
            "only the code carrying this attempt's nonce may be accepted"
        );
    }

    #[test]
    fn a_nonce_is_unique_per_attempt() {
        assert_ne!(new_nonce(), new_nonce());
        assert_eq!(new_nonce().len(), 64);
    }
}

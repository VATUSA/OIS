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

        let url = format!(
            "{}/api/v1/auth/vatsim/login?desktop=true&return_to={}",
            api_base.trim_end_matches('/'),
            urlencode(LOOPBACK_REDIRECT)
        );
        open::that(&url).map_err(|e| format!("could not open your browser: {e}"))?;

        wait_for_code(&listener)
    })
    .await
    .map_err(|e| format!("sign-in task failed: {e}"))?
}

/// Accepts loopback connections until one carries a `code`, or we give up.
///
/// A browser may open more than one connection (a speculative one, a favicon fetch), so anything
/// without a code is answered and ignored rather than treated as the answer.
fn wait_for_code(listener: &TcpListener) -> Result<String, String> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 2048];
                let read = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);

                match request_code(&request) {
                    Some(code) => {
                        respond(
                            &mut stream,
                            "Signed in. You can close this tab and return to OIS.",
                        );
                        return Ok(code);
                    }
                    None => respond(&mut stream, "Waiting for sign-in..."),
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

/// Pulls `code` out of the request line (`GET /callback?code=… HTTP/1.1`).
fn request_code(request: &str) -> Option<String> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let query = target.split_once('?')?.1;

    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "code" && !value.is_empty()).then(|| urldecode(value))
    })
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
fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

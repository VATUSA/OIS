//! The one API origin the desktop shell sends the session token to or opens a sign-in URL on (#924).
//!
//! Rust owns it rather than taking it from the webview: a script injected into the webview could
//! otherwise name any host, and these requests leave from Rust, outside the CSP's `connect-src`. The
//! origin is compiled in from `VITE_OIS_API_URL`, the variable `desktop/scripts/pin-release-api.py`
//! exports for the release build, so in a release the webview's API base and this one come from one
//! value. Cargo reads only the shell environment, never `web/.env.local`, so a dev build can disagree
//! with the webview (`desktop/README.md`).

use reqwest::Url;

/// The local backend a debug build may talk to (`just backend`).
pub const DEV_ORIGIN: &str = "http://127.0.0.1:3000";

/// What one build of the app may talk to.
pub struct Build {
    /// The origin pinned at compile time, if any.
    pub pinned: Option<&'static str>,
    /// A debug build also allows [`DEV_ORIGIN`].
    pub debug: bool,
}

/// This binary's build. A const rather than `#[cfg]` branches, so CI's debug builds compile and test
/// the release rule too.
pub const THIS_BUILD: Build = Build {
    pinned: option_env!("VITE_OIS_API_URL"),
    debug: cfg!(debug_assertions),
};

impl Build {
    /// Whether `origin` may receive the token or a sign-in URL. A release build allows exactly its
    /// pinned origin, which must be `https`; a debug build also allows [`DEV_ORIGIN`]. Either is
    /// matched as a bare origin: one trailing slash at most, no userinfo, path, query or fragment.
    pub fn allows(&self, origin: &str) -> bool {
        let Some(origin) = bare_origin(origin) else {
            return false;
        };
        let pinned = self.pin().and_then(bare_origin);
        let pinned_https =
            pinned.is_some_and(|p| p.scheme() == "https" && p.origin() == origin.origin());
        let dev =
            self.debug && bare_origin(DEV_ORIGIN).is_some_and(|d| d.origin() == origin.origin());
        pinned_https || dev
    }

    /// The origin the commands build their URLs on, without a trailing slash: the pin, or
    /// [`DEV_ORIGIN`] in a debug build with nothing pinned. Refused (and logged) when [`Self::allows`]
    /// says no; a refused pin never falls back to the dev origin.
    pub fn api_origin(&self) -> Result<String, String> {
        let candidate = match (self.pin(), self.debug) {
            (Some(pin), _) => pin,
            (None, true) => DEV_ORIGIN,
            (None, false) => {
                log::warn!("refusing to contact OIS: this release build has no API origin pinned");
                return Err("This build of OIS has no server configured.".into());
            }
        };
        if self.allows(candidate) {
            return Ok(candidate.trim_end_matches('/').to_string());
        }
        log::warn!(
            "refusing to contact OIS at {candidate:?}: not an allowed API origin for this build"
        );
        Err(format!(
            "This build of OIS is not allowed to contact {candidate}."
        ))
    }

    /// The pin, with a blank `VITE_OIS_API_URL` read as unset.
    fn pin(&self) -> Option<&'static str> {
        self.pinned.map(str::trim).filter(|pin| !pin.is_empty())
    }
}

/// `text` parsed as an `http(s)` origin with nothing after it but one optional `/`, else `None`.
fn bare_origin(text: &str) -> Option<Url> {
    let url = Url::parse(text).ok()?;
    let bare = matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    bare.then_some(url)
}

#[cfg(test)]
mod tests {
    //! Every test builds an explicit `Build`. None reads `THIS_BUILD`: the local gate compiles with
    //! `VITE_OIS_API_URL=http://127.0.0.1:3000` from `.env`, CI with it unset.

    use std::sync::{Mutex, OnceLock};

    use super::*;

    const PIN: &str = "https://ois.example.org";

    fn release(pinned: Option<&'static str>) -> Build {
        Build {
            pinned,
            debug: false,
        }
    }

    fn debug(pinned: Option<&'static str>) -> Build {
        Build {
            pinned,
            debug: true,
        }
    }

    fn assert_refuses(build: &Build, origins: &[&str]) {
        for origin in origins {
            assert!(!build.allows(origin), "{origin} must be refused");
        }
    }

    // ---- allows(): AC4's list ----

    #[test]
    fn a_release_build_allows_its_pinned_https_origin() {
        assert!(release(Some(PIN)).allows(PIN));
    }

    #[test]
    fn a_release_build_allows_the_pin_with_one_trailing_slash() {
        assert!(release(Some(PIN)).allows("https://ois.example.org/"));
        assert!(release(Some("https://ois.example.org/")).allows(PIN));
        assert!(
            !release(Some(PIN)).allows("https://ois.example.org//"),
            "a second slash is a path"
        );
    }

    #[test]
    fn a_debug_build_allows_the_dev_origin_with_one_trailing_slash() {
        assert!(debug(None).allows("http://127.0.0.1:3000/"));
    }

    #[test]
    fn a_release_build_refuses_a_foreign_https_host() {
        let build = release(Some(PIN));
        assert!(build.allows(PIN), "positive control");
        assert_refuses(
            &build,
            &[
                "https://evil.test",
                "https://ois.example.org.evil.test",
                "https://evil-ois.example.org",
                "https://api.ois.example.org",
            ],
        );
    }

    #[test]
    fn a_release_build_refuses_http_to_a_non_loopback_host() {
        for build in [release(Some(PIN)), debug(Some(PIN))] {
            assert_refuses(&build, &["http://ois.example.org", "http://evil.test"]);
        }
    }

    #[test]
    fn a_release_build_refuses_file_and_custom_schemes() {
        assert_refuses(
            &debug(Some(PIN)),
            &[
                "file:///etc/passwd",
                "ois://ois.example.org",
                "wss://ois.example.org",
                "javascript:alert(1)",
            ],
        );
    }

    #[test]
    fn an_origin_with_a_path_is_refused() {
        let build = release(Some(PIN));
        assert!(build.allows(PIN), "positive control");
        assert_refuses(
            &build,
            &[
                "https://ois.example.org/api",
                "https://ois.example.org/api/v1/auth/vatsim/login",
            ],
        );
    }

    #[test]
    fn an_origin_with_a_query_is_refused() {
        let build = release(Some(PIN));
        assert!(build.allows("https://ois.example.org/"), "positive control");
        assert_refuses(
            &build,
            &[
                "https://ois.example.org?x=1",
                "https://ois.example.org/?x=1",
            ],
        );
    }

    #[test]
    fn an_origin_with_userinfo_is_refused() {
        let build = release(Some(PIN));
        assert!(build.allows(PIN), "positive control");
        assert_refuses(
            &build,
            &[
                "https://user@ois.example.org",
                "https://user:pw@ois.example.org",
                "https://ois.example.org@evil.test",
            ],
        );
    }

    #[test]
    fn an_origin_with_a_fragment_is_refused() {
        assert_refuses(
            &release(Some(PIN)),
            &["https://ois.example.org#x", "https://ois.example.org/#x"],
        );
    }

    #[test]
    fn an_empty_or_unparseable_origin_is_refused() {
        assert_refuses(
            &debug(Some(PIN)),
            &["", "not a url", "ois.example.org", "https://"],
        );
    }

    #[test]
    fn the_default_https_port_is_the_same_origin_as_the_pin() {
        assert!(release(Some(PIN)).allows("https://ois.example.org:443"));
    }

    #[test]
    fn a_non_default_port_is_a_different_origin() {
        assert!(!release(Some(PIN)).allows("https://ois.example.org:8443"));
        assert!(!debug(None).allows("http://127.0.0.1:80"));
    }

    // ---- allows(): AC2, the build rules ----

    #[test]
    fn a_release_build_with_no_pin_refuses_everything() {
        assert_refuses(&release(None), &[DEV_ORIGIN, "http://127.0.0.1:3000/", PIN]);
    }

    #[test]
    fn a_release_build_with_a_blank_pin_refuses_everything() {
        assert_refuses(&release(Some("")), &["", DEV_ORIGIN, PIN]);
    }

    #[test]
    fn a_release_build_refuses_an_http_pin() {
        assert!(!release(Some("http://ois.example.org")).allows("http://ois.example.org"));
        // What `.env` pins locally: a release never reaches the dev origin.
        assert!(!release(Some(DEV_ORIGIN)).allows(DEV_ORIGIN));
    }

    #[test]
    fn a_release_build_refuses_a_pin_with_a_path() {
        assert_refuses(
            &release(Some("https://ois.example.org/api")),
            &["https://ois.example.org/api", PIN],
        );
    }

    #[test]
    fn a_release_build_refuses_the_dev_origin_beside_a_valid_pin() {
        assert!(!release(Some(PIN)).allows(DEV_ORIGIN));
        assert!(debug(Some(PIN)).allows(DEV_ORIGIN), "positive control");
    }

    #[test]
    fn a_debug_build_with_no_pin_allows_the_dev_origin() {
        assert_eq!(DEV_ORIGIN, "http://127.0.0.1:3000");
        assert!(debug(None).allows("http://127.0.0.1:3000"));
        assert!(!debug(None).allows(PIN), "no pin, nothing else");
    }

    #[test]
    fn a_debug_build_with_an_https_pin_allows_both() {
        let build = debug(Some(PIN));
        assert!(build.allows(PIN));
        assert!(build.allows("http://127.0.0.1:3000"));
        assert!(!build.allows("https://evil.test"));
    }

    #[test]
    fn a_debug_build_refuses_other_loopback_origins() {
        let build = debug(None);
        assert!(build.allows("http://127.0.0.1:3000"), "positive control");
        assert_refuses(
            &build,
            &[
                "http://localhost:3000",
                "http://127.0.0.1:3001",
                "http://127.0.0.1",
                "https://127.0.0.1:3000",
                "http://[::1]:3000",
                "http://127.0.0.2:3000",
            ],
        );
    }

    #[test]
    fn a_debug_build_refuses_the_dev_origin_with_a_path_or_query() {
        assert_refuses(
            &debug(None),
            &[
                "http://127.0.0.1:3000/api",
                "http://127.0.0.1:3000?x=1",
                "http://user@127.0.0.1:3000",
                "http://127.0.0.1:3000#x",
            ],
        );
    }

    // ---- api_origin(): the value the commands build their URLs on ----

    #[test]
    fn api_origin_returns_the_pin_without_its_trailing_slash() {
        assert_eq!(
            release(Some("https://ois.example.org/")).api_origin(),
            Ok(PIN.to_string())
        );
        assert_eq!(release(Some(PIN)).api_origin(), Ok(PIN.to_string()));
    }

    #[test]
    fn api_origin_falls_back_to_the_dev_origin_in_a_debug_build() {
        assert_eq!(
            debug(None).api_origin(),
            Ok("http://127.0.0.1:3000".to_string())
        );
        assert_eq!(
            debug(Some(" ")).api_origin(),
            Ok("http://127.0.0.1:3000".to_string()),
            "blank is unset"
        );
    }

    #[test]
    fn api_origin_prefers_the_pin_in_a_debug_build() {
        assert_eq!(debug(Some(PIN)).api_origin(), Ok(PIN.to_string()));
    }

    #[test]
    fn api_origin_refuses_a_release_build_with_no_pin() {
        let refused = release(None).api_origin();
        assert!(
            refused.as_ref().is_err_and(|m| !m.is_empty()),
            "{refused:?}"
        );
    }

    #[test]
    fn api_origin_refusal_names_the_refused_origin() {
        let refused = release(Some("http://evil.test")).api_origin();
        assert!(
            refused
                .as_ref()
                .is_err_and(|m| m.contains("http://evil.test")),
            "{refused:?}"
        );
    }

    #[test]
    fn api_origin_does_not_fall_back_to_the_dev_origin_on_a_bad_pin() {
        let refused = debug(Some("http://evil.test")).api_origin();
        assert!(
            refused
                .as_ref()
                .is_err_and(|m| m.contains("http://evil.test")),
            "{refused:?}"
        );
    }

    // ---- AC3: logging ----

    struct Capture(Mutex<Vec<(log::Level, String)>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            self.0
                .lock()
                .unwrap()
                .push((record.level(), record.args().to_string()));
        }
        fn flush(&self) {}
    }

    /// The process's logger. `cargo test` runs every test in one process, so each test reads only the
    /// records naming an origin no other test uses.
    fn captured() -> &'static Capture {
        static LOGGER: OnceLock<&'static Capture> = OnceLock::new();
        LOGGER.get_or_init(|| {
            let logger: &'static Capture = Box::leak(Box::new(Capture(Mutex::new(Vec::new()))));
            log::set_logger(logger).expect("no other logger in a test process");
            log::set_max_level(log::LevelFilter::Trace);
            logger
        })
    }

    /// Every record naming `origin`, at any level.
    fn records_naming(origin: &str) -> Vec<(log::Level, String)> {
        let records = captured().0.lock().unwrap();
        records
            .iter()
            .filter(|(_, m)| m.contains(origin))
            .cloned()
            .collect()
    }

    #[test]
    fn a_refused_origin_is_logged_at_warn_naming_the_origin() {
        const REFUSED: &str = "http://refused-and-logged.test";
        captured();
        assert!(release(Some(REFUSED)).api_origin().is_err());
        let records = records_naming(REFUSED);
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].0, log::Level::Warn, "{records:?}");
    }

    #[test]
    fn an_allowed_origin_is_not_logged() {
        const ALLOWED: &str = "https://allowed-and-silent.test";
        captured();
        assert!(release(Some(ALLOWED)).api_origin().is_ok());
        let records = records_naming(ALLOWED);
        assert!(records.is_empty(), "{records:?}");
    }

    // ---- AC2: the build-time pin is the one the release script writes ----

    /// The variable name is a contract between two files no test runs together.
    #[test]
    fn this_build_reads_the_variable_the_release_script_writes() {
        let script = include_str!("../../scripts/pin-release-api.py");
        let (_, written) = script
            .split_once("f.write(f\"")
            .expect("the script's env-file write");
        let (name, _) = written.split_once("={origin}").expect("NAME={origin}");
        let source = include_str!("api_origin.rs");
        assert!(
            source.contains(&format!("option_env!(\"{name}\")")),
            "THIS_BUILD must read {name}"
        );
        assert_eq!(THIS_BUILD.debug, cfg!(debug_assertions));
    }
}
